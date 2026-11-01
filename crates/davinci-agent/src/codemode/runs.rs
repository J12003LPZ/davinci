//! Durable, host-owned Script run records. Guest code never writes these
//! fields and never supplies a handle it can forge.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScriptRunState {
    Created,
    Running,
    Completed,
    Failed,
    Cancelled,
    Stalled,
    RecoveryRequired,
}

impl ScriptRunState {
    pub fn terminal(self) -> bool {
        !matches!(self, Self::Created | Self::Running)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScriptRunRecord {
    pub run_id: String,
    pub outer_call_id: String,
    pub session_id: Option<String>,
    pub workspace_binding: String,
    pub permission_revision: String,
    pub started_at_ms: u64,
    pub state: ScriptRunState,
    pub log_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptRunError {
    pub code: String,
    pub message: String,
}

impl ScriptRunRecord {
    pub fn new(
        run_id: &str,
        outer_call_id: &str,
        session_id: Option<&str>,
        workspace_binding: &str,
        permission_revision: &str,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            outer_call_id: outer_call_id.into(),
            session_id: session_id.map(str::to_owned),
            workspace_binding: workspace_binding.into(),
            permission_revision: permission_revision.into(),
            started_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_millis() as u64)
                .unwrap_or_default(),
            state: ScriptRunState::Created,
            log_ref: None,
        }
    }
}

pub struct ScriptRunRegistry {
    dir: PathBuf,
}

impl ScriptRunRegistry {
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, ScriptRunError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)
            .map_err(|_| ScriptRunError::new("UNAVAILABLE", "script run store unavailable"))?;
        Ok(Self { dir })
    }

    /// Write-ahead creation: the record is durable before the script runs.
    pub fn create(&self, record: &ScriptRunRecord) -> Result<(), ScriptRunError> {
        let path = self.path(&record.run_id)?;
        if path.exists() {
            return Err(ScriptRunError::new("DENIED", "script run already exists"));
        }
        self.write(&path, record)
    }

    pub fn get(&self, run_id: &str) -> Result<ScriptRunRecord, ScriptRunError> {
        let path = self.path(run_id)?;
        let bytes = std::fs::read(&path)
            .map_err(|_| ScriptRunError::new("UNAVAILABLE", "script run unavailable"))?;
        serde_json::from_slice(&bytes)
            .map_err(|_| ScriptRunError::new("PROTOCOL_ERROR", "malformed script run record"))
    }

    /// Resolve a handle only for the session and workspace that own it.
    pub fn get_for(
        &self,
        run_id: &str,
        session_id: Option<&str>,
        workspace_binding: &str,
    ) -> Result<ScriptRunRecord, ScriptRunError> {
        let record = self.get(run_id)?;
        if record.session_id.as_deref() != session_id
            || record.workspace_binding != workspace_binding
        {
            return Err(ScriptRunError::new(
                "DENIED",
                "script run belongs to another session or workspace",
            ));
        }
        Ok(record)
    }

    /// Advance the state monotonically. A repeated terminal state is a
    /// suppressed duplicate (`false`); going backwards is refused.
    pub fn transition(&self, run_id: &str, next: ScriptRunState) -> Result<bool, ScriptRunError> {
        let mut record = self.get(run_id)?;
        if record.state == next && next.terminal() {
            return Ok(false);
        }
        let allowed = match (record.state, next) {
            (ScriptRunState::Created, ScriptRunState::Running) => true,
            (ScriptRunState::Running, to) => to.terminal(),
            (ScriptRunState::RecoveryRequired, to) => matches!(
                to,
                ScriptRunState::Completed | ScriptRunState::Failed | ScriptRunState::Stalled
            ),
            (from, to) => from.terminal() && from == to,
        };
        if !allowed {
            return Err(ScriptRunError::new(
                "DENIED",
                "script run state transition is not monotonic",
            ));
        }
        record.state = next;
        self.write(&self.path(run_id)?, &record)?;
        Ok(true)
    }

    fn path(&self, run_id: &str) -> Result<PathBuf, ScriptRunError> {
        if run_id.is_empty()
            || run_id.len() > 128
            || !run_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(ScriptRunError::new(
                "INVALID_INPUT",
                "invalid script run id",
            ));
        }
        Ok(Path::new(&self.dir).join(format!("{run_id}.json")))
    }

    fn write(&self, path: &Path, record: &ScriptRunRecord) -> Result<(), ScriptRunError> {
        use std::io::Write;
        let bytes = serde_json::to_vec(record)
            .map_err(|_| ScriptRunError::new("PROTOCOL_ERROR", "invalid script run record"))?;
        let temp = path.with_extension("json.tmp");
        let mut file = std::fs::File::create(&temp)
            .map_err(|_| ScriptRunError::new("UNAVAILABLE", "script run write failed"))?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|_| ScriptRunError::new("UNAVAILABLE", "script run write failed"))?;
        std::fs::rename(&temp, path)
            .map_err(|_| ScriptRunError::new("UNAVAILABLE", "script run write failed"))
    }
}

/// Result of reconciling one durable run after a host restart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciledScriptRun {
    pub run_id: String,
    pub state: ScriptRunState,
    /// True when this call changed the stored state.
    pub recovered: bool,
    pub reason: Option<String>,
}

/// Reconcile a run against the fact that this host process is new. Guest
/// JavaScript is never resumed: a run that was still open becomes
/// `RecoveryRequired`, and a terminal run is left exactly as recorded.
pub fn reconcile_script_run(
    registry: &ScriptRunRegistry,
    run_id: &str,
) -> Result<ReconciledScriptRun, ScriptRunError> {
    let mut record = registry.get(run_id)?;
    if record.state.terminal() {
        // RecoveryRequired may resolve through the operation journal, but this
        // function never invents an outcome.
        return Ok(ReconciledScriptRun {
            run_id: record.run_id,
            state: record.state,
            recovered: false,
            reason: None,
        });
    }
    record.state = ScriptRunState::RecoveryRequired;
    registry.write_record(&record)?;
    Ok(ReconciledScriptRun {
        run_id: record.run_id,
        state: record.state,
        recovered: true,
        reason: Some(
            "host restarted while the script was open; guest JavaScript is not resumed".into(),
        ),
    })
}

impl ScriptRunRegistry {
    fn write_record(&self, record: &ScriptRunRecord) -> Result<(), ScriptRunError> {
        self.write(&self.path(&record.run_id)?, record)
    }
}

impl ScriptRunError {
    fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}
