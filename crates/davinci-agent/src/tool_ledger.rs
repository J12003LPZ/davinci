//! Exactly-once tool-call ledger matching §9.
//! Prevents duplicate side effects during transport recovery, reconnects, or continuation replay.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::runtime::operations::{digest_bytes, LegacyObservation, LegacySourceKind};
use crate::runtime::{conservative_replay_policy, ReplayPolicy};

const MAX_STORED_OUTPUT: usize = 64 * 1024;
const MAX_TERMINAL_RECORDS: usize = 256;
const MAX_JOURNAL_BYTES: u64 = 48 * 1024 * 1024;
const OUTPUT_TRUNCATED_MARKER: &str = "\n[output truncated in ledger; digest covers the full text]";

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

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum LedgerJournalEvent {
    Upsert { record: ToolCallRecord },
    Remove { call_id: String },
}

fn bounded_output(output: &str) -> String {
    if output.len() <= MAX_STORED_OUTPUT {
        return output.to_string();
    }

    let mut prefix_end = MAX_STORED_OUTPUT - OUTPUT_TRUNCATED_MARKER.len();
    while !output.is_char_boundary(prefix_end) {
        prefix_end -= 1;
    }
    let mut stored = String::with_capacity(MAX_STORED_OUTPUT);
    stored.push_str(&output[..prefix_end]);
    stored.push_str(OUTPUT_TRUNCATED_MARKER);
    stored
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
    davinci_sys::fs::atomic_write(path, bytes).map_err(|err| err.to_string())

}

fn journal_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("ledger"))
        .to_string_lossy();
    path.with_file_name(format!("{name}.events.jsonl"))
}

fn read_journal(
    path: &Path,
    repair_torn_tail: bool,
) -> Result<(Vec<LedgerJournalEvent>, Vec<u8>), String> {
    let mut bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), Vec::new()))
        }
        Err(error) => return Err(error.to_string()),
    };
    let complete_len = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    if repair_torn_tail && complete_len < bytes.len() {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(|error| error.to_string())?;
        file.set_len(complete_len as u64)
            .map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        bytes.truncate(complete_len);
    }
    let mut events = Vec::new();
    for line in bytes[..complete_len].split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        events.push(
            serde_json::from_slice(line)
                .map_err(|error| format!("tool ledger journal is corrupt: {error}"))?,
        );
    }
    Ok((events, bytes))
}

fn append_journal(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() {
        return Ok(());
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    #[cfg(unix)]
    std::fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| error.to_string())?;
    Ok(())
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
    #[serde(skip, default)]
    persistence_error: Option<String>,
    #[serde(skip, default)]
    dirty_call_ids: HashSet<String>,
    #[serde(skip, default)]
    snapshot_metadata_dirty: bool,
    #[serde(skip, default)]
    persistence_bytes_written: u64,
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
            persistence_error: None,
            dirty_call_ids: HashSet::new(),
            snapshot_metadata_dirty: false,
            persistence_bytes_written: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolCallExecutionAuthority {
    CompatibilityLedger,
    OperationJournal,
    LegacyObservationBlocked,
}

impl ToolCallLedger {
    /// Read a legacy ledger without reconciling or rewriting it. This is the
    /// migration boundary: the original bytes provide the source digest and
    /// each record is imported as an observation before restart cleanup can
    /// remove compatibility entries.
    pub fn legacy_observations_from_path(
        path: &Path,
        session_id: &str,
    ) -> Result<Vec<LegacyObservation>, String> {
        let snapshot_exists = path.is_file();
        let snapshot_bytes = if snapshot_exists {
            std::fs::read(path).map_err(|error| error.to_string())?
        } else {
            Vec::new()
        };
        let (events, journal_bytes) = read_journal(&journal_path(path), false)?;
        if !snapshot_exists && journal_bytes.is_empty() {
            return Ok(Vec::new());
        }
        let mut ledger: Self = if snapshot_exists {
            serde_json::from_slice(&snapshot_bytes)
                .map_err(|error| format!("tool ledger is corrupt: {error}"))?
        } else {
            Self::new(session_id, session_id)
        };
        if !ledger.session_id.is_empty() && ledger.session_id != session_id {
            return Err("tool ledger belongs to a different session".into());
        }
        ledger.apply_journal_events(events)?;
        ledger.prune_terminal_records();
        let source_identity = std::fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .into_owned();
        let source_digest = if journal_bytes.is_empty() {
            digest_bytes(&snapshot_bytes)
        } else {
            let mut source_bytes =
                Vec::with_capacity(snapshot_bytes.len() + journal_bytes.len() + 8);
            source_bytes.extend_from_slice(&(snapshot_bytes.len() as u64).to_le_bytes());
            source_bytes.extend_from_slice(&snapshot_bytes);
            source_bytes.extend_from_slice(&journal_bytes);
            digest_bytes(&source_bytes)
        };
        let mut records = Vec::with_capacity(ledger.records.len());
        let mut ordered_ids = ledger.record_order.clone();
        let ordered_set: std::collections::HashSet<_> = ordered_ids.iter().cloned().collect();
        let mut unordered_ids: Vec<_> = ledger
            .records
            .keys()
            .filter(|id| !ordered_set.contains(*id))
            .cloned()
            .collect();
        unordered_ids.sort();
        ordered_ids.extend(unordered_ids);
        for call_id in ordered_ids {
            let Some(record) = ledger.records.get(&call_id) else {
                return Err("tool ledger record order references a missing record".into());
            };
            let mut observation = LegacyObservation::new(
                LegacySourceKind::ToolLedger,
                source_identity.clone(),
                source_digest.clone(),
                record.call_id.clone(),
                serde_json::to_value(record.status)
                    .map_err(|error| error.to_string())?
                    .as_str()
                    .unwrap_or("unknown"),
            )
            .map_err(|error| error.to_string())?;
            observation
                .evidence_provenance
                .push("tool_ledger.status".into());
            if let Some(result_digest) = &record.result_digest {
                observation.known_result = Some(serde_json::json!({
                    "result_digest": result_digest,
                    "is_error": record.is_error,
                }));
                observation
                    .evidence_provenance
                    .push("tool_ledger.result_digest".into());
            }
            observation.observed_at_ms = record.executed_at;
            observation.validate().map_err(|error| error.to_string())?;
            records.push(observation);
        }
        Ok(records)
    }

    pub(crate) fn execution_authority(
        &self,
        call_id: &str,
        builtin_capability: bool,
        journal_configured: bool,
    ) -> ToolCallExecutionAuthority {
        if builtin_capability && journal_configured {
            if self.records.contains_key(call_id) {
                ToolCallExecutionAuthority::LegacyObservationBlocked
            } else {
                ToolCallExecutionAuthority::OperationJournal
            }
        } else {
            ToolCallExecutionAuthority::CompatibilityLedger
        }
    }

    pub fn new(session_id: impl Into<String>, lineage_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            lineage_id: lineage_id.into(),
            records: HashMap::new(),
            record_order: Vec::new(),
            condvar: default_condvar(),
            persistence_path: None,
            persistence_error: None,
            dirty_call_ids: HashSet::new(),
            snapshot_metadata_dirty: false,
            persistence_bytes_written: 0,
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
        let metadata_changed = ledger.session_id != session_id || ledger.lineage_id.is_empty();
        ledger.session_id = session_id.to_string();
        if ledger.lineage_id.is_empty() {
            ledger.lineage_id = session_id.to_string();
        }
        let (events, _) = read_journal(&journal_path(path), true)?;
        ledger.apply_journal_events(events)?;
        ledger.dirty_call_ids.clear();
        ledger.snapshot_metadata_dirty = metadata_changed;
        ledger.persistence_path = Some(path.to_path_buf());
        ledger.prune_terminal_records();
        ledger.reconcile_after_restart();
        ledger.persist()?;
        Ok(ledger)
    }

    fn apply_journal_events(&mut self, events: Vec<LedgerJournalEvent>) -> Result<(), String> {
        for event in events {
            match event {
                LedgerJournalEvent::Upsert { record } => {
                    if record.call_id.is_empty() {
                        return Err("tool ledger journal contains an empty call id".into());
                    }
                    if !self.records.contains_key(&record.call_id) {
                        self.record_order.push(record.call_id.clone());
                    }
                    self.records.insert(record.call_id.clone(), record);
                }
                LedgerJournalEvent::Remove { call_id } => {
                    self.records.remove(&call_id);
                    self.record_order
                        .retain(|recorded_id| recorded_id != &call_id);
                }
            }
        }
        Ok(())
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
                        self.dirty_call_ids.insert(id.clone());
                    }
                }
            }
        }
        for id in &remove {
            self.records.remove(id);
            self.dirty_call_ids.insert(id.clone());
        }
        self.record_order.retain(|id| self.records.contains_key(id));
    }

    pub fn persist(&mut self) -> Result<(), String> {
        self.ensure_durable()?;
        let Some(path) = self.persistence_path.clone() else {
            return Ok(());
        };
        let result = self.persist_changes(&path);
        if let Err(error) = &result {
            self.fail_persistence(format!("Tool ledger persistence failed: {error}"));
        }
        self.ensure_durable()
    }

    fn persist_changes(&mut self, path: &Path) -> Result<(), String> {
        let journal = journal_path(path);
        if !path.is_file() || self.snapshot_metadata_dirty {
            return self.compact_snapshot(path, &journal);
        }

        let mut event_bytes = Vec::new();
        for call_id in self.ordered_record_ids() {
            if self.dirty_call_ids.contains(&call_id) {
                let record = self
                    .records
                    .get(&call_id)
                    .expect("ordered ids only contain current records");
                serde_json::to_writer(
                    &mut event_bytes,
                    &LedgerJournalEvent::Upsert {
                        record: record.clone(),
                    },
                )
                .map_err(|error| error.to_string())?;
                event_bytes.push(b'\n');
            }
        }
        let mut removed: Vec<_> = self
            .dirty_call_ids
            .iter()
            .filter(|call_id| !self.records.contains_key(*call_id))
            .cloned()
            .collect();
        removed.sort();
        for call_id in removed {
            serde_json::to_writer(&mut event_bytes, &LedgerJournalEvent::Remove { call_id })
                .map_err(|error| error.to_string())?;
            event_bytes.push(b'\n');
        }

        if !event_bytes.is_empty() {
            append_journal(&journal, &event_bytes)?;
            self.persistence_bytes_written = self
                .persistence_bytes_written
                .saturating_add(event_bytes.len() as u64);
            self.dirty_call_ids.clear();
        }

        let journal_len = match std::fs::metadata(&journal) {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error.to_string()),
        };
        if journal_len >= MAX_JOURNAL_BYTES {
            self.compact_snapshot(path, &journal)?;
        }
        Ok(())
    }

    fn compact_snapshot(&mut self, path: &Path, journal: &Path) -> Result<(), String> {
        let snapshot = serde_json::to_vec(self).map_err(|error| error.to_string())?;
        atomic_write_json(path, &snapshot)?;
        self.persistence_bytes_written = self
            .persistence_bytes_written
            .saturating_add(snapshot.len() as u64);

        match std::fs::metadata(journal) {
            Ok(metadata) if metadata.len() > 0 => atomic_write_json(journal, &[])?,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
        self.dirty_call_ids.clear();
        self.snapshot_metadata_dirty = false;
        Ok(())
    }

    pub(crate) fn fail_persistence(&mut self, error: String) {
        if self.persistence_error.is_none() {
            self.persistence_error = Some(format!(
                "{error}. Reopen and reconcile the session before dispatching more tools."
            ));
        }
        self.condvar.notify_all();
    }

    fn ensure_durable(&self) -> Result<(), String> {
        match &self.persistence_error {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }

    fn ordered_record_ids(&self) -> Vec<String> {
        let mut seen = HashSet::with_capacity(self.records.len());
        let mut ordered = Vec::with_capacity(self.records.len());
        for id in &self.record_order {
            if self.records.contains_key(id) && seen.insert(id.as_str()) {
                ordered.push(id.clone());
            }
        }

        let mut untracked: Vec<_> = self
            .records
            .values()
            .filter(|record| !seen.contains(record.call_id.as_str()))
            .collect();
        untracked.sort_by(|left, right| {
            left.executed_at
                .cmp(&right.executed_at)
                .then_with(|| left.call_id.cmp(&right.call_id))
        });
        ordered.extend(untracked.into_iter().map(|record| record.call_id.clone()));
        ordered
    }

    fn prune_terminal_records(&mut self) {
        let terminal: Vec<_> = self
            .ordered_record_ids()
            .into_iter()
            .filter(|id| {
                self.records.get(id).is_some_and(|record| {
                    matches!(
                        record.status,
                        ToolExecutionStatus::Completed
                            | ToolExecutionStatus::Failed
                            | ToolExecutionStatus::Blocked
                    )
                })
            })
            .collect();
        let excess = terminal.len().saturating_sub(MAX_TERMINAL_RECORDS);
        for id in terminal.into_iter().take(excess) {
            self.records.remove(&id);
            self.dirty_call_ids.insert(id);
        }
        let records = &self.records;
        let mut seen = HashSet::with_capacity(self.record_order.len());
        self.record_order
            .retain(|id| records.contains_key(id) && seen.insert(id.clone()));
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
        self.ensure_durable()?;
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
        self.dirty_call_ids.insert(call_id.to_string());
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
        if let Err(error) = self.ensure_durable() {
            return BeginOutcome::ReplayBlocked(error);
        }
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
                    self.dirty_call_ids.insert(call_id.to_string());
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
            self.dirty_call_ids.insert(call_id.to_string());
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
        self.ensure_durable()?;
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
                    self.dirty_call_ids.insert(call_id.to_string());
                    Ok(RecoveryAction::Retry)
                }
            }
            AttemptOutcome::StartedUnknown => match record.replay_policy {
                ReplayPolicy::SafeToReplay => {
                    record.status = ToolExecutionStatus::Pending;
                    self.dirty_call_ids.insert(call_id.to_string());
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
        self.dirty_call_ids.insert(call_id.to_string());
        Ok(())
    }

    pub fn cancel_reservation(&mut self, call_id: &str) {
        if let Some(rec) = self.records.get(call_id) {
            if rec.status == ToolExecutionStatus::Pending {
                self.records.remove(call_id);
                self.dirty_call_ids.insert(call_id.to_string());
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
            guard.ensure_durable()?;
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
                let changed = entry.status == ToolExecutionStatus::Pending;
                entry.status = ToolExecutionStatus::Executing;
                if changed {
                    self.dirty_call_ids.insert(call_id.to_string());
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
            self.dirty_call_ids.insert(call_id.to_string());
        }
        let _ = self.persist();
    }

    pub fn record_completion(&mut self, call_id: &str, output: &str, is_error: bool) {
        if let Some(entry) = self.records.get_mut(call_id) {
            entry.status = ToolExecutionStatus::Completed;
            entry.outcome = AttemptOutcome::Succeeded;
            entry.output = Some(bounded_output(output));
            entry.result_digest = Some(compute_digest(output));
            entry.is_error = is_error;
            entry.executed_at = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            );
            self.dirty_call_ids.insert(call_id.to_string());
        }
        self.prune_terminal_records();
        self.condvar.notify_all();
    }

    pub fn record_failure(&mut self, call_id: &str, error: &str) {
        if let Some(entry) = self.records.get_mut(call_id) {
            entry.status = ToolExecutionStatus::Failed;
            entry.outcome = AttemptOutcome::Failed;
            entry.output = Some(bounded_output(error));
            entry.result_digest = Some(compute_digest(error));
            entry.is_error = true;
            self.dirty_call_ids.insert(call_id.to_string());
        }
        self.prune_terminal_records();
        self.condvar.notify_all();
    }

    pub fn record_blocked(&mut self, call_id: &str, reason: &str) {
        if let Some(entry) = self.records.get_mut(call_id) {
            entry.status = ToolExecutionStatus::Blocked;
            entry.output = Some(bounded_output(reason));
            entry.is_error = true;
            self.dirty_call_ids.insert(call_id.to_string());
        }
        self.prune_terminal_records();
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
    fn configured_operation_journal_blocks_legacy_record_fallback() {
        let mut ledger = ToolCallLedger::new("session", "lineage");
        ledger.record_start("legacy-call", "edit", &json!({"path": "file"}));
        assert_eq!(
            ledger.execution_authority("legacy-call", true, true),
            ToolCallExecutionAuthority::LegacyObservationBlocked
        );
        assert_eq!(
            ledger.execution_authority("new-call", true, true),
            ToolCallExecutionAuthority::OperationJournal
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
    fn completion_output_is_bounded_without_splitting_utf8() {
        let mut ledger = ToolCallLedger::new("sess_output", "lin_output");
        let call_id = "call_large_output";
        let output = "🙂".repeat(20_000);
        ledger.record_start(call_id, "read", &json!({"path": "large.txt"}));
        ledger.record_completion(call_id, &output, false);

        let record = ledger.records().get(call_id).unwrap();
        let stored = record.output.as_deref().unwrap();
        let marker = "\n[output truncated in ledger; digest covers the full text]";
        assert!(stored.len() <= 64 * 1024, "{} bytes", stored.len());
        assert!(stored.ends_with(marker));
        assert!(stored.is_char_boundary(stored.len() - marker.len()));
        assert_eq!(
            record.result_digest.as_deref(),
            Some(compute_digest(&output).as_str())
        );
    }

    #[test]
    fn ledger_size_stays_bounded_over_a_long_session() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.json");
        let mut ledger = ToolCallLedger::load_bound(&path, "session-bounded").unwrap();
        let output = "y".repeat(20_000);

        ledger.record_start("reserved", "write", &json!({"path": "pending.txt"}));
        for i in 0..2_000 {
            let id = format!("call-{i}");
            let arguments = json!({"path": i});
            assert_eq!(
                ledger.reserve_call(&id, "read", &arguments).unwrap(),
                ReservationOutcome::Reserved
            );
            assert_eq!(
                ledger.begin_execution(&id, "read", &arguments),
                BeginOutcome::Execute
            );
            ledger.persist().unwrap();
            ledger.record_completion(&id, &output, false);
            ledger.persist().unwrap();
        }
        ledger.persist().unwrap();

        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.len() < 8 * 1024 * 1024, "{} bytes", bytes.len());
        assert!(!bytes.contains(&b'\n'), "ledger JSON should be compact");
        let journal_bytes = std::fs::metadata(journal_path(&path)).unwrap().len();
        assert!(
            bytes.len() as u64 + journal_bytes < 50_000_000,
            "{} total persisted bytes",
            bytes.len() as u64 + journal_bytes
        );
        assert!(
            ledger.persistence_bytes_written < 50_000_000,
            "{} cumulative ledger bytes written",
            ledger.persistence_bytes_written
        );
        assert_eq!(ledger.records().len(), 257);
        assert!(ledger.records().contains_key("reserved"));
        assert!(!ledger.records().contains_key("call-1743"));
        assert!(ledger.records().contains_key("call-1744"));
        assert!(!ledger.records().contains_key("call-0"));
        assert!(matches!(
            ledger.begin_execution("call-0", "read", &json!({"path": 0})),
            BeginOutcome::Execute
        ));

        let restored = ToolCallLedger::load_bound(&path, "session-bounded").unwrap();
        assert_eq!(restored.records().len(), 257);
        assert_eq!(
            restored.records()["call-1999"].output.as_deref(),
            Some(output.as_str())
        );
        assert!(!restored.records().contains_key("call-0"));
    }

    #[test]
    fn ledger_persistence_write_budget_scales_with_call_output() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.json");
        let mut ledger = ToolCallLedger::load_bound(&path, "session-write-budget").unwrap();
        let output = "y".repeat(20_000);

        for i in 0..20 {
            let id = format!("budget-call-{i}");
            let arguments = json!({"path": i});
            assert_eq!(
                ledger.reserve_call(&id, "read", &arguments).unwrap(),
                ReservationOutcome::Reserved
            );
            assert_eq!(
                ledger.begin_execution(&id, "read", &arguments),
                BeginOutcome::Execute
            );
            ledger.persist().unwrap();
            ledger.record_completion(&id, &output, false);
            ledger.persist().unwrap();
        }

        assert!(
            ledger.persistence_bytes_written < 1_000_000,
            "{} ledger bytes written for 20 calls",
            ledger.persistence_bytes_written
        );
    }

    #[test]
    fn ledger_journal_replays_for_legacy_observations_and_repairs_torn_tail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.json");
        let mut ledger = ToolCallLedger::load_bound(&path, "session-journal").unwrap();
        let id = "journal-call";
        ledger.record_start(id, "read", &json!({"path": "notes.txt"}));
        ledger.record_completion(id, "read result", false);
        ledger.persist().unwrap();

        let observations =
            ToolCallLedger::legacy_observations_from_path(&path, "session-journal").unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].record_identity, id);
        assert_eq!(observations[0].state, "completed");
        assert!(observations[0]
            .known_result
            .as_ref()
            .is_some_and(|result| result["is_error"] == false));

        let journal = journal_path(&path);
        let mut torn = std::fs::OpenOptions::new()
            .append(true)
            .open(&journal)
            .unwrap();
        torn.write_all(b"{\"event\":\"upsert\"").unwrap();
        drop(torn);

        let restored = ToolCallLedger::load_bound(&path, "session-journal").unwrap();
        assert_eq!(
            restored.records()[id].output.as_deref(),
            Some("read result")
        );
        assert!(std::fs::read(&journal).unwrap().ends_with(b"\n"));
    }

    #[test]
    fn ledger_journal_compacts_into_snapshot_and_continues_appending() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.json");
        let journal = journal_path(&path);
        let mut ledger = ToolCallLedger::load_bound(&path, "session-compact").unwrap();

        ledger.record_start("first", "read", &json!({"path": "first.txt"}));
        ledger.record_completion("first", "first result", false);
        ledger.persist().unwrap();
        assert!(std::fs::metadata(&journal).unwrap().len() > 0);

        let checkpoint = serde_json::to_vec(&ledger).unwrap();
        atomic_write_json(&path, &checkpoint).unwrap();
        let mut restored = ToolCallLedger::load_bound(&path, "session-compact").unwrap();
        assert_eq!(
            restored.records()["first"].output.as_deref(),
            Some("first result")
        );

        restored.compact_snapshot(&path, &journal).unwrap();
        assert_eq!(std::fs::metadata(&journal).unwrap().len(), 0);
        restored.record_start("second", "read", &json!({"path": "second.txt"}));
        restored.record_completion("second", "second result", false);
        restored.persist().unwrap();

        let restored = ToolCallLedger::load_bound(&path, "session-compact").unwrap();
        assert_eq!(
            restored.records()["second"].output.as_deref(),
            Some("second result")
        );
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
