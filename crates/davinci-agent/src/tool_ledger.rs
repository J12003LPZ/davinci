//! Exactly-once tool-call ledger matching §9.
//! Prevents duplicate side effects during transport recovery, reconnects, or continuation replay.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::runtime::{conservative_replay_policy, ReplayPolicy};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSideEffect {
    ReadOnly,
    Mutating,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionStatus {
    Pending,
    Executing,
    Completed,
    Failed,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptOutcome {
    NotStarted,
    StartedUnknown,
    Succeeded,
    Failed,
}

impl Default for AttemptOutcome {
    fn default() -> Self {
        Self::NotStarted
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryAction {
    Execute,
    Retry,
    Replay { output: String, is_error: bool },
    ReconcileBeforeRetry(String),
    Stop(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub call_id: String,
    pub tool_name: String,
    pub normalized_arguments: Value,
    pub argument_digest: String,
    pub side_effect: ToolSideEffect,
    #[serde(default)]
    pub replay_policy: ReplayPolicy,
    #[serde(default)]
    pub outcome: AttemptOutcome,
    #[serde(default)]
    pub pre_state_hash: Option<String>,
    #[serde(default)]
    pub post_state_hash: Option<String>,
    pub status: ToolExecutionStatus,
    pub result_digest: Option<String>,
    pub output: Option<String>,
    pub is_error: bool,
    pub executed_at: Option<u64>,
}

impl ToolCallRecord {
    pub fn attempt_id(&self) -> &str {
        &self.call_id
    }
}

pub fn classify_side_effect(tool_name: &str) -> ToolSideEffect {
    match tool_name {
        "read" | "grep" | "find" | "ls" | "web_fetch" | "web_search" | "job_output"
        | "mcp_read" | "tool_search" => ToolSideEffect::ReadOnly,
        _ => ToolSideEffect::Mutating,
    }
}

pub fn normalize_arguments(args: &Value) -> Value {
    match args {
        Value::Object(map) => {
            let mut sorted: Vec<(&String, &Value)> = map.iter().collect();
            sorted.sort_by_key(|(k, _)| *k);
            let mut obj = serde_json::Map::new();
            for (k, v) in sorted {
                obj.insert(k.clone(), normalize_arguments(v));
            }
            Value::Object(obj)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(normalize_arguments).collect()),
        other => other.clone(),
    }
}

pub fn canonical_arguments_digest(args: &Value) -> String {
    let normalized = normalize_arguments(args);
    let s = serde_json::to_string(&normalized).unwrap_or_default();
    compute_digest(&s)
}

fn compute_digest(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReservationOutcome {
    /// Reserved as Pending. Caller is the leader and must execute it.
    Reserved,
    /// Terminal result already exists. Caller should replay cached result.
    Replay { output: String, is_error: bool },
    /// A terminal result exists, but replay requires explicit reconciliation.
    ReplayBlocked(String),
    /// An identical call is currently in-flight (Pending or Executing). Caller is a follower and must wait.
    WaitForInFlight,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BeginOutcome {
    /// Proceed to execute the tool call as leader.
    Execute,
    /// Terminal result already exists.
    Replay { output: String, is_error: bool },
    /// A terminal result exists, but replay requires explicit reconciliation.
    ReplayBlocked(String),
    /// An identical call is already executing on another thread; wait for it.
    WaitForInFlight,
    /// Call ID was previously used with different tool name or arguments.
    Collision(String),
}

fn atomic_write_json(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "tool ledger path has no parent directory".to_string())?;
    std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("ledger");
    let temp = parent.join(format!(
        ".{name}.tmp-{}-{}",
        std::process::id(),
        now_millis()
    ));
    let write_result = (|| -> Result<(), String> {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|err| err.to_string())?;
        file.write_all(bytes).map_err(|err| err.to_string())?;
        file.sync_all().map_err(|err| err.to_string())?;

        #[cfg(windows)]
        if path.exists() {
            let previous = parent.join(format!(".{name}.previous"));
            let _ = std::fs::remove_file(&previous);
            std::fs::rename(path, &previous).map_err(|err| err.to_string())?;
            if let Err(error) = std::fs::rename(&temp, path) {
                let _ = std::fs::rename(&previous, path);
                return Err(error.to_string());
            }
            let _ = std::fs::remove_file(previous);
        }
        #[cfg(not(windows))]
        std::fs::rename(&temp, path).map_err(|err| err.to_string())?;
        #[cfg(windows)]
        if !path.exists() {
            std::fs::rename(&temp, path).map_err(|err| err.to_string())?;
        }

        if let Ok(directory) = std::fs::File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    write_result
}

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn default_condvar() -> Arc<Condvar> {
    Arc::new(Condvar::new())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallLedger {
    pub session_id: String,
    pub lineage_id: String,
    records: HashMap<String, ToolCallRecord>,
    #[serde(default)]
    record_order: Vec<String>,
    #[serde(skip, default = "default_condvar")]
    condvar: Arc<Condvar>,
    #[serde(skip, default)]
    persistence_path: Option<PathBuf>,
}

impl Default for ToolCallLedger {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            lineage_id: String::new(),
            records: HashMap::new(),
            record_order: Vec::new(),
            condvar: default_condvar(),
            persistence_path: None,
        }
    }
}

impl ToolCallLedger {
    pub fn new(session_id: impl Into<String>, lineage_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            lineage_id: lineage_id.into(),
            records: HashMap::new(),
            record_order: Vec::new(),
            condvar: default_condvar(),
            persistence_path: None,
        }
    }

    pub fn load_bound(path: &Path, session_id: &str) -> Result<Self, String> {
        let mut ledger = if path.is_file() {
            let bytes = std::fs::read(path).map_err(|err| err.to_string())?;
            serde_json::from_slice::<Self>(&bytes)
                .map_err(|err| format!("tool ledger is corrupt: {err}"))?
        } else {
            Self::new(session_id, session_id)
        };
        if !ledger.session_id.is_empty() && ledger.session_id != session_id {
            return Err("tool ledger belongs to a different session".into());
        }
        ledger.session_id = session_id.to_string();
        if ledger.lineage_id.is_empty() {
            ledger.lineage_id = session_id.to_string();
        }
        ledger.persistence_path = Some(path.to_path_buf());
        ledger.reconcile_after_restart();
        ledger.persist()?;
        Ok(ledger)
    }

    fn reconcile_after_restart(&mut self) {
        let mut remove = Vec::new();
        for (id, record) in &mut self.records {
            if record.outcome == AttemptOutcome::NotStarted
                && record.status == ToolExecutionStatus::Pending
            {
                remove.push(id.clone());
                continue;
            }
            if record.outcome == AttemptOutcome::StartedUnknown
                || record.status == ToolExecutionStatus::Executing
            {
                match record.replay_policy {
                    ReplayPolicy::SafeToReplay => remove.push(id.clone()),
                    ReplayPolicy::ReconcileBeforeReplay | ReplayPolicy::NeverAutoReplay => {
                        record.status = ToolExecutionStatus::Blocked;
                        record.output = Some(replay_blocked_message(
                            &record.tool_name,
                            record.replay_policy,
                        ));
                        record.is_error = true;
                    }
                }
            }
        }
        for id in &remove {
            self.records.remove(id);
        }
        self.record_order.retain(|id| self.records.contains_key(id));
    }

    pub fn persist(&self) -> Result<(), String> {
        let Some(path) = &self.persistence_path else {
            return Ok(());
        };
        let bytes = serde_json::to_vec_pretty(self).map_err(|err| err.to_string())?;
        atomic_write_json(path, &bytes)
    }

    pub fn records(&self) -> &HashMap<String, ToolCallRecord> {
        &self.records
    }

    pub fn recent_tool_names(&self, limit: usize) -> Vec<String> {
        if self.record_order.is_empty() {
            let mut legacy_records = self.records.values().collect::<Vec<_>>();
            legacy_records.sort_by(|left, right| {
                right
                    .executed_at
                    .cmp(&left.executed_at)
                    .then_with(|| right.call_id.cmp(&left.call_id))
            });
            return legacy_records
                .into_iter()
                .take(limit)
                .map(|record| record.tool_name.clone())
                .collect();
        }

        self.record_order
            .iter()
            .rev()
            .filter_map(|call_id| self.records.get(call_id))
            .take(limit)
            .map(|record| record.tool_name.clone())
            .collect()
    }

    pub fn is_already_executed(&self, call_id: &str) -> bool {
        self.records
            .get(call_id)
            .is_some_and(|r| r.status == ToolExecutionStatus::Completed)
    }

    pub fn get_completed_result(&self, call_id: &str) -> Option<(String, bool)> {
        let rec = self.records.get(call_id)?;
        if rec.status == ToolExecutionStatus::Completed {
            rec.output.clone().map(|out| (out, rec.is_error))
        } else {
            None
        }
    }

    /// Retrieve completed/terminal result matching the given call ID, tool name, and arguments.
    /// Returns Err if the call ID was recorded with differing tool name or arguments.
    pub fn get_completed_result_matching(
        &self,
        call_id: &str,
        tool_name: &str,
        arguments: &Value,
    ) -> Result<Option<(String, bool)>, String> {
        let Some(rec) = self.records.get(call_id) else {
            return Ok(None);
        };
        if rec.tool_name != tool_name {
            return Err(format!(
                "Tool call id collision for `{call_id}`: previously registered for tool `{}`, but requested for `{tool_name}`",
                rec.tool_name
            ));
        }
        let norm_args = normalize_arguments(arguments);
        let arg_digest = canonical_arguments_digest(arguments);
        if rec.argument_digest != arg_digest && rec.normalized_arguments != norm_args {
            return Err(format!(
                "Tool call id collision for `{call_id}`: arguments differ from prior call"
            ));
        }
        if rec.status == ToolExecutionStatus::Completed {
            Ok(rec.output.clone().map(|out| (out, rec.is_error)))
        } else if rec.status == ToolExecutionStatus::Failed
            || rec.status == ToolExecutionStatus::Blocked
        {
            Ok(rec.output.clone().map(|out| (out, true)))
        } else {
            Ok(None)
        }
    }

    /// Atomically check or reserve a tool call in the ledger.
    pub fn reserve_call(
        &mut self,
        call_id: &str,
        tool_name: &str,
        arguments: &Value,
    ) -> Result<ReservationOutcome, String> {
        self.reserve_call_with_policy(
            call_id,
            tool_name,
            arguments,
            conservative_replay_policy(tool_name),
        )
    }

    /// Atomically check or reserve a tool call using the runtime's replay
    /// policy. The policy is persisted with a new record so recovery does not
    /// silently become more permissive when a registry changes.
    pub fn reserve_call_with_policy(
        &mut self,
        call_id: &str,
        tool_name: &str,
        arguments: &Value,
        replay_policy: ReplayPolicy,
    ) -> Result<ReservationOutcome, String> {
        self.reserve_call_with_metadata(
            call_id,
            tool_name,
            arguments,
            replay_policy,
            classify_side_effect(tool_name),
        )
    }

    pub fn reserve_call_with_metadata(
        &mut self,
        call_id: &str,
        tool_name: &str,
        arguments: &Value,
        replay_policy: ReplayPolicy,
        side_effect: ToolSideEffect,
    ) -> Result<ReservationOutcome, String> {
        let norm_args = normalize_arguments(arguments);
        let arg_digest = canonical_arguments_digest(arguments);
        if let Some(rec) = self.records.get(call_id) {
            if rec.tool_name != tool_name {
                return Err(format!(
                    "Tool call id collision for `{call_id}`: previously registered for tool `{}`, but requested for `{tool_name}`",
                    rec.tool_name
                ));
            }
            if rec.argument_digest != arg_digest && rec.normalized_arguments != norm_args {
                return Err(format!(
                    "Tool call id collision for `{call_id}`: arguments differ from prior call"
                ));
            }
            match rec.status {
                ToolExecutionStatus::Completed
                | ToolExecutionStatus::Failed
                | ToolExecutionStatus::Blocked => {
                    return Ok(match rec.replay_policy {
                        ReplayPolicy::SafeToReplay => ReservationOutcome::Replay {
                            output: rec.output.clone().unwrap_or_default(),
                            is_error: rec.is_error,
                        },
                        ReplayPolicy::ReconcileBeforeReplay | ReplayPolicy::NeverAutoReplay => {
                            ReservationOutcome::ReplayBlocked(replay_blocked_message(
                                &rec.tool_name,
                                rec.replay_policy,
                            ))
                        }
                    });
                }
                ToolExecutionStatus::Pending | ToolExecutionStatus::Executing => {
                    return Ok(ReservationOutcome::WaitForInFlight);
                }
            }
        }
        self.record_order.push(call_id.to_string());
        self.records.insert(
            call_id.to_string(),
            ToolCallRecord {
                call_id: call_id.to_string(),
                tool_name: tool_name.to_string(),
                normalized_arguments: norm_args,
                argument_digest: arg_digest,
                side_effect,
                replay_policy,
                outcome: AttemptOutcome::NotStarted,
                pre_state_hash: None,
                post_state_hash: None,
                status: ToolExecutionStatus::Pending,
                result_digest: None,
                output: None,
                is_error: false,
                executed_at: None,
            },
        );
        self.persist()
            .map_err(|err| format!("tool ledger persistence failed: {err}"))?;
        Ok(ReservationOutcome::Reserved)
    }

    /// Mark an execution as beginning, or return Replay / WaitForInFlight / Collision.
    pub fn begin_execution(
        &mut self,
        call_id: &str,
        tool_name: &str,
        arguments: &Value,
    ) -> BeginOutcome {
        self.begin_execution_with_policy(
            call_id,
            tool_name,
            arguments,
            conservative_replay_policy(tool_name),
        )
    }

    /// Mark an execution as beginning using an authoritative replay policy.
    pub fn begin_execution_with_policy(
        &mut self,
        call_id: &str,
        tool_name: &str,
        arguments: &Value,
        replay_policy: ReplayPolicy,
    ) -> BeginOutcome {
        let norm_args = normalize_arguments(arguments);
        let arg_digest = canonical_arguments_digest(arguments);
        if let Some(rec) = self.records.get_mut(call_id) {
            if rec.tool_name != tool_name {
                return BeginOutcome::Collision(format!(
                    "Tool call id collision for `{call_id}`: previously registered for tool `{}`, but requested for `{tool_name}`",
                    rec.tool_name
                ));
            }
            if rec.argument_digest != arg_digest && rec.normalized_arguments != norm_args {
                return BeginOutcome::Collision(format!(
                    "Tool call id collision for `{call_id}`: arguments differ from prior call"
                ));
            }
            match rec.status {
                ToolExecutionStatus::Completed
                | ToolExecutionStatus::Failed
                | ToolExecutionStatus::Blocked => match rec.replay_policy {
                    ReplayPolicy::SafeToReplay => BeginOutcome::Replay {
                        output: rec.output.clone().unwrap_or_default(),
                        is_error: rec.is_error,
                    },
                    ReplayPolicy::ReconcileBeforeReplay | ReplayPolicy::NeverAutoReplay => {
                        BeginOutcome::ReplayBlocked(replay_blocked_message(
                            &rec.tool_name,
                            rec.replay_policy,
                        ))
                    }
                },
                ToolExecutionStatus::Executing => BeginOutcome::WaitForInFlight,
                ToolExecutionStatus::Pending => {
                    rec.status = ToolExecutionStatus::Executing;
                    rec.outcome = AttemptOutcome::StartedUnknown;
                    BeginOutcome::Execute
                }
            }
        } else {
            self.record_order.push(call_id.to_string());
            self.records.insert(
                call_id.to_string(),
                ToolCallRecord {
                    call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    normalized_arguments: norm_args,
                    argument_digest: arg_digest,
                    side_effect: classify_side_effect(tool_name),
                    replay_policy,
                    outcome: AttemptOutcome::StartedUnknown,
                    pre_state_hash: None,
                    post_state_hash: None,
                    status: ToolExecutionStatus::Executing,
                    result_digest: None,
                    output: None,
                    is_error: false,
                    executed_at: None,
                },
            );
            BeginOutcome::Execute
        }
    }

    /// Decide how a persisted attempt may proceed after a crash, timeout, or
    /// lost response. A retry decision moves the record back to Pending so
    /// the caller can pass it through the normal begin/execute path.
    pub fn recover_attempt(
        &mut self,
        call_id: &str,
        tool_name: &str,
        arguments: &Value,
        retries_remaining: u32,
    ) -> Result<RecoveryAction, String> {
        let norm_args = normalize_arguments(arguments);
        let arg_digest = canonical_arguments_digest(arguments);
        let record = self
            .records
            .get_mut(call_id)
            .ok_or_else(|| format!("Tool call `{call_id}` is not present in the ledger"))?;
        if record.tool_name != tool_name {
            return Err(format!(
                "Tool call id collision for `{call_id}`: previously registered for tool `{}`, but requested for `{tool_name}`",
                record.tool_name
            ));
        }
        if record.argument_digest != arg_digest && record.normalized_arguments != norm_args {
            return Err(format!(
                "Tool call id collision for `{call_id}`: arguments differ from prior call"
            ));
        }
        if record.status == ToolExecutionStatus::Blocked {
            return Ok(RecoveryAction::Stop(
                "The tool call was blocked and requires a new authorized call".into(),
            ));
        }

        let outcome = match (record.outcome, record.status) {
            (AttemptOutcome::NotStarted, ToolExecutionStatus::Executing) => {
                AttemptOutcome::StartedUnknown
            }
            (AttemptOutcome::NotStarted, ToolExecutionStatus::Completed) => {
                AttemptOutcome::Succeeded
            }
            (AttemptOutcome::NotStarted, ToolExecutionStatus::Failed) => AttemptOutcome::Failed,
            (outcome, _) => outcome,
        };

        match outcome {
            AttemptOutcome::NotStarted => Ok(RecoveryAction::Execute),
            AttemptOutcome::Succeeded => match record.output.clone() {
                Some(output) => Ok(RecoveryAction::Replay {
                    output,
                    is_error: record.is_error,
                }),
                None => Ok(RecoveryAction::Stop(
                    "A successful tool attempt has no persisted result".into(),
                )),
            },
            AttemptOutcome::Failed => {
                if retries_remaining == 0 {
                    Ok(RecoveryAction::Stop(
                        "The tool attempt failed and its retry budget is exhausted".into(),
                    ))
                } else {
                    record.status = ToolExecutionStatus::Pending;
                    Ok(RecoveryAction::Retry)
                }
            }
            AttemptOutcome::StartedUnknown => match record.replay_policy {
                ReplayPolicy::SafeToReplay => {
                    record.status = ToolExecutionStatus::Pending;
                    Ok(RecoveryAction::Retry)
                }
                ReplayPolicy::ReconcileBeforeReplay => Ok(RecoveryAction::ReconcileBeforeRetry(
                    replay_blocked_message(&record.tool_name, record.replay_policy),
                )),
                ReplayPolicy::NeverAutoReplay => Ok(RecoveryAction::Stop(replay_blocked_message(
                    &record.tool_name,
                    record.replay_policy,
                ))),
            },
        }
    }

    pub fn set_state_hashes(
        &mut self,
        call_id: &str,
        pre_state_hash: Option<String>,
        post_state_hash: Option<String>,
    ) -> Result<(), String> {
        let record = self
            .records
            .get_mut(call_id)
            .ok_or_else(|| format!("Tool call `{call_id}` is not present in the ledger"))?;
        record.pre_state_hash = pre_state_hash;
        record.post_state_hash = post_state_hash;
        Ok(())
    }

    pub fn cancel_reservation(&mut self, call_id: &str) {
        if let Some(rec) = self.records.get(call_id) {
            if rec.status == ToolExecutionStatus::Pending {
                self.records.remove(call_id);
                self.record_order
                    .retain(|recorded_id| recorded_id != call_id);
                self.condvar.notify_all();
            }
        }
    }

    /// Wait for an in-flight tool call to reach a terminal status (Completed, Failed, Blocked).
    pub fn wait_for_terminal(
        ledger: &Arc<Mutex<ToolCallLedger>>,
        call_id: &str,
        abort: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<(String, bool), String> {
        let mut guard = ledger.lock().unwrap_or_else(|e| e.into_inner());
        let cv = guard.condvar.clone();
        let deadline = std::time::Instant::now() + Duration::from_secs(600);
        loop {
            if abort.is_some_and(|f| f.load(std::sync::atomic::Ordering::SeqCst)) {
                return Err(format!("Aborted while waiting for tool call `{call_id}`"));
            }
            if std::time::Instant::now() >= deadline {
                return Err(format!("Timed out waiting for tool call `{call_id}`"));
            }
            let Some(rec) = guard.records.get(call_id) else {
                return Err(format!("Tool call `{call_id}` not found in ledger"));
            };
            match rec.status {
                ToolExecutionStatus::Completed
                | ToolExecutionStatus::Failed
                | ToolExecutionStatus::Blocked => {
                    return Ok((rec.output.clone().unwrap_or_default(), rec.is_error));
                }
                ToolExecutionStatus::Pending | ToolExecutionStatus::Executing => {
                    guard = match cv.wait_timeout(guard, Duration::from_millis(50)) {
                        Ok((g, _)) => g,
                        Err(e) => e.into_inner().0,
                    };
                }
            }
        }
    }

    pub fn record_start(&mut self, call_id: &str, tool_name: &str, arguments: &Value) {
        self.record_start_with_policy(
            call_id,
            tool_name,
            arguments,
            conservative_replay_policy(tool_name),
        );
    }

    pub fn record_start_with_policy(
        &mut self,
        call_id: &str,
        tool_name: &str,
        arguments: &Value,
        replay_policy: ReplayPolicy,
    ) {
        let norm_args = normalize_arguments(arguments);
        let arg_digest = canonical_arguments_digest(arguments);
        if let Some(entry) = self.records.get_mut(call_id) {
            // Preserve terminal results without allowing record_start to overwrite them!
            if matches!(
                entry.status,
                ToolExecutionStatus::Pending | ToolExecutionStatus::Executing
            ) {
                entry.status = ToolExecutionStatus::Executing;
            }
        } else {
            self.record_order.push(call_id.to_string());
            self.records.insert(
                call_id.to_string(),
                ToolCallRecord {
                    call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    normalized_arguments: norm_args,
                    argument_digest: arg_digest,
                    side_effect: classify_side_effect(tool_name),
                    replay_policy,
                    outcome: AttemptOutcome::StartedUnknown,
                    pre_state_hash: None,
                    post_state_hash: None,
                    status: ToolExecutionStatus::Executing,
                    result_digest: None,
                    output: None,
                    is_error: false,
                    executed_at: None,
                },
            );
        }
        let _ = self.persist();
    }

    pub fn record_completion(&mut self, call_id: &str, output: &str, is_error: bool) {
        if let Some(entry) = self.records.get_mut(call_id) {
            entry.status = ToolExecutionStatus::Completed;
            entry.outcome = AttemptOutcome::Succeeded;
            entry.output = Some(output.to_string());
            entry.result_digest = Some(compute_digest(output));
            entry.is_error = is_error;
            entry.executed_at = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            );
        }
        self.condvar.notify_all();
    }

    pub fn record_failure(&mut self, call_id: &str, error: &str) {
        if let Some(entry) = self.records.get_mut(call_id) {
            entry.status = ToolExecutionStatus::Failed;
            entry.outcome = AttemptOutcome::Failed;
            entry.output = Some(error.to_string());
            entry.result_digest = Some(compute_digest(error));
            entry.is_error = true;
        }
        self.condvar.notify_all();
    }

    pub fn record_blocked(&mut self, call_id: &str, reason: &str) {
        if let Some(entry) = self.records.get_mut(call_id) {
            entry.status = ToolExecutionStatus::Blocked;
            entry.output = Some(reason.to_string());
            entry.is_error = true;
        }
        self.condvar.notify_all();
    }
}

fn replay_blocked_message(tool_name: &str, replay_policy: ReplayPolicy) -> String {
    format!(
        "Tool call `{tool_name}` cannot be replayed automatically; replay policy is {replay_policy:?}. Reconcile the tool state before retrying."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn uncertain_safe_read_can_be_replayed() {
        let mut ledger = ToolCallLedger::new("sess_recovery", "lin_recovery");
        let args = json!({"path": "README.md"});
        ledger.record_start_with_policy("call_read", "read", &args, ReplayPolicy::SafeToReplay);

        let action = ledger
            .recover_attempt("call_read", "read", &args, 0)
            .unwrap();

        assert_eq!(action, RecoveryAction::Retry);
        assert_eq!(
            ledger.begin_execution_with_policy(
                "call_read",
                "read",
                &args,
                ReplayPolicy::SafeToReplay,
            ),
            BeginOutcome::Execute
        );
    }

    #[test]
    fn uncertain_mutation_requires_reconciliation() {
        let mut ledger = ToolCallLedger::new("sess_recovery", "lin_recovery");
        let args = json!({"path": "src/main.rs", "oldText": "old", "newText": "new"});
        ledger.record_start_with_policy(
            "call_edit",
            "edit",
            &args,
            ReplayPolicy::ReconcileBeforeReplay,
        );

        let action = ledger
            .recover_attempt("call_edit", "edit", &args, 1)
            .unwrap();

        assert!(matches!(
            action,
            RecoveryAction::ReconcileBeforeRetry(message) if message.contains("Reconcile")
        ));
        assert_eq!(
            ledger.records().get("call_edit").unwrap().status,
            ToolExecutionStatus::Executing
        );
    }

    #[test]
    fn confirmed_failure_can_retry_within_budget() {
        let mut ledger = ToolCallLedger::new("sess_recovery", "lin_recovery");
        let args = json!({"path": "README.md"});
        ledger.record_start_with_policy("call_failed", "read", &args, ReplayPolicy::SafeToReplay);
        ledger.record_failure("call_failed", "temporary read failure");

        let action = ledger
            .recover_attempt("call_failed", "read", &args, 1)
            .unwrap();

        assert_eq!(action, RecoveryAction::Retry);
        assert_eq!(
            ledger.begin_execution_with_policy(
                "call_failed",
                "read",
                &args,
                ReplayPolicy::SafeToReplay,
            ),
            BeginOutcome::Execute
        );
    }

    #[test]
    fn attempt_metadata_round_trips_through_ledger_serialization() {
        let mut ledger = ToolCallLedger::new("sess_recovery", "lin_recovery");
        let args = json!({"path": "src/main.rs"});
        ledger.record_start_with_policy(
            "call_hashes",
            "edit",
            &args,
            ReplayPolicy::ReconcileBeforeReplay,
        );
        ledger
            .set_state_hashes("call_hashes", Some("before".into()), Some("after".into()))
            .unwrap();

        let encoded = serde_json::to_string(&ledger).unwrap();
        let restored: ToolCallLedger = serde_json::from_str(&encoded).unwrap();
        let record = restored.records().get("call_hashes").unwrap();
        assert_eq!(record.attempt_id(), "call_hashes");
        assert_eq!(record.outcome, AttemptOutcome::StartedUnknown);
        assert_eq!(record.replay_policy, ReplayPolicy::ReconcileBeforeReplay);
        assert_eq!(record.pre_state_hash.as_deref(), Some("before"));
        assert_eq!(record.post_state_hash.as_deref(), Some("after"));
    }

    #[test]
    fn classifies_side_effects_correctly() {
        assert_eq!(classify_side_effect("read"), ToolSideEffect::ReadOnly);
        assert_eq!(classify_side_effect("grep"), ToolSideEffect::ReadOnly);
        assert_eq!(classify_side_effect("find"), ToolSideEffect::ReadOnly);
        assert_eq!(classify_side_effect("ls"), ToolSideEffect::ReadOnly);
        assert_eq!(
            classify_side_effect("apply_patch"),
            ToolSideEffect::Mutating
        );
        assert_eq!(classify_side_effect("bash"), ToolSideEffect::Mutating);
        assert_eq!(
            classify_side_effect("exec_command"),
            ToolSideEffect::Mutating
        );
    }

    #[test]
    fn prevents_duplicate_execution_on_retry() {
        let mut ledger = ToolCallLedger::new("sess_1", "lin_1");
        let call_id = "call_abc123";
        assert!(!ledger.is_already_executed(call_id));

        ledger.record_start(call_id, "apply_patch", &json!({"input": "*** Begin Patch"}));
        assert!(!ledger.is_already_executed(call_id));

        ledger.record_completion(call_id, "Applied patch", false);
        assert!(ledger.is_already_executed(call_id));

        let (result, is_err) = ledger.get_completed_result(call_id).unwrap();
        assert_eq!(result, "Applied patch");
        assert!(!is_err);
    }

    #[test]
    fn reused_call_id_different_tool_fails_with_collision_error() {
        let mut ledger = ToolCallLedger::new("sess_1", "lin_1");
        let call_id = "call_reuse_1";
        ledger.record_start(call_id, "apply_patch", &json!({"input": "patch 1"}));
        ledger.record_completion(call_id, "Patch applied", false);

        // Attempting to reserve the same call_id with a different tool name must fail with collision error
        let err = ledger
            .reserve_call(call_id, "bash", &json!({"command": "echo hi"}))
            .unwrap_err();
        assert!(err.contains("collision"), "{err}");
        assert!(err.contains("apply_patch"), "{err}");
        assert!(err.contains("bash"), "{err}");

        // get_completed_result_matching also rejects
        let lookup_err = ledger
            .get_completed_result_matching(call_id, "bash", &json!({"command": "echo hi"}))
            .unwrap_err();
        assert!(lookup_err.contains("collision"), "{lookup_err}");
    }

    #[test]
    fn reused_call_id_different_args_fails_with_collision_error() {
        let mut ledger = ToolCallLedger::new("sess_1", "lin_1");
        let call_id = "call_reuse_2";
        ledger.record_start(call_id, "bash", &json!({"command": "echo one"}));
        ledger.record_completion(call_id, "one\n", false);

        // Attempting to reserve the same call_id with different args must fail with collision error
        let err = ledger
            .reserve_call(call_id, "bash", &json!({"command": "echo two"}))
            .unwrap_err();
        assert!(err.contains("collision"), "{err}");
        assert!(err.contains("arguments differ"), "{err}");

        // Matching args remain blocked because shell execution is never safe
        // to replay automatically.
        let res = ledger
            .reserve_call(call_id, "bash", &json!({"command": "echo one"}))
            .unwrap();
        assert!(matches!(
            res,
            ReservationOutcome::ReplayBlocked(message) if message.contains("replay")
        ));
    }

    #[test]
    fn canonical_arguments_order_independent() {
        let mut ledger = ToolCallLedger::new("sess_1", "lin_1");
        let call_id = "call_canon";
        let args1 = json!({"a": 1, "b": 2});
        let args2 = json!({"b": 2, "a": 1});
        ledger.record_start(call_id, "read", &args1);
        ledger.record_completion(call_id, "done", false);

        let res = ledger.reserve_call(call_id, "read", &args2).unwrap();
        assert_eq!(
            res,
            ReservationOutcome::Replay {
                output: "done".into(),
                is_error: false
            }
        );
    }

    #[test]
    fn terminal_record_not_overwritten_by_record_start() {
        let mut ledger = ToolCallLedger::new("sess_1", "lin_1");
        let call_id = "call_terminal";
        ledger.record_start(call_id, "bash", &json!({"command": "ls"}));
        ledger.record_completion(call_id, "file.txt\n", false);

        // A second record_start must NOT overwrite the completed status or output
        ledger.record_start(call_id, "bash", &json!({"command": "ls"}));
        let (result, is_err) = ledger.get_completed_result(call_id).unwrap();
        assert_eq!(result, "file.txt\n");
        assert!(!is_err);
    }

    #[test]
    fn recent_tool_names_returns_newest_calls_first() {
        let mut ledger = ToolCallLedger::new("sess_recent", "lin_recent");
        for (index, tool) in ["read", "grep", "bash", "edit", "ls", "write"]
            .into_iter()
            .enumerate()
        {
            ledger.record_start(
                &format!("call_recent_{index}"),
                tool,
                &json!({"index": index}),
            );
        }

        assert_eq!(
            ledger.recent_tool_names(3),
            vec!["write".to_string(), "ls".to_string(), "edit".to_string()]
        );
    }

    #[test]
    fn concurrent_wait_for_terminal_receives_result() {
        let ledger = Arc::new(Mutex::new(ToolCallLedger::new("sess_1", "lin_1")));
        let call_id = "call_concurrent";

        let outcome = ledger
            .lock()
            .unwrap()
            .reserve_call(call_id, "bash", &json!({"command": "echo hi"}))
            .unwrap();
        assert_eq!(outcome, ReservationOutcome::Reserved);

        let outcome2 = ledger
            .lock()
            .unwrap()
            .reserve_call(call_id, "bash", &json!({"command": "echo hi"}))
            .unwrap();
        assert_eq!(outcome2, ReservationOutcome::WaitForInFlight);

        // Spawn a thread waiting on the in-flight call
        let l2 = Arc::clone(&ledger);
        let handle =
            std::thread::spawn(move || ToolCallLedger::wait_for_terminal(&l2, call_id, None));

        // Small sleep to ensure follower thread is waiting
        std::thread::sleep(Duration::from_millis(50));

        // Leader completes
        {
            let mut l = ledger.lock().unwrap();
            l.record_start(call_id, "bash", &json!({"command": "echo hi"}));
            l.record_completion(call_id, "hi\n", false);
        }

        let res = handle.join().unwrap().unwrap();
        assert_eq!(res, ("hi\n".to_string(), false));
    }

    #[test]
    fn tool_ledger_atomic_persist_replaces_complete_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.json");
        atomic_write_json(&path, br#"{"generation":1}"#).unwrap();
        atomic_write_json(&path, br#"{"generation":2,"complete":true}"#).unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["generation"], 2);
        assert_eq!(value["complete"], true);
        assert!(std::fs::read_dir(dir.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".tmp-")
        }));
    }

    #[test]
    fn corrupt_tool_ledger_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tool-ledger.json");
        std::fs::write(&path, b"{not-json").unwrap();
        let error = ToolCallLedger::load_bound(&path, "session-a").unwrap_err();
        assert!(error.contains("tool ledger is corrupt"));
    }
}
