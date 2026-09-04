//! Exactly-once tool-call ledger matching §9.
//! Prevents duplicate side effects during transport recovery, reconnects, or continuation replay.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub call_id: String,
    pub tool_name: String,
    pub normalized_arguments: Value,
    pub argument_digest: String,
    pub side_effect: ToolSideEffect,
    pub status: ToolExecutionStatus,
    pub result_digest: Option<String>,
    pub output: Option<String>,
    pub is_error: bool,
    pub executed_at: Option<u64>,
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
    /// An identical call is currently in-flight (Pending or Executing). Caller is a follower and must wait.
    WaitForInFlight,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BeginOutcome {
    /// Proceed to execute the tool call as leader.
    Execute,
    /// Terminal result already exists.
    Replay { output: String, is_error: bool },
    /// An identical call is already executing on another thread; wait for it.
    WaitForInFlight,
    /// Call ID was previously used with different tool name or arguments.
    Collision(String),
}

fn default_condvar() -> Arc<Condvar> {
    Arc::new(Condvar::new())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallLedger {
    pub session_id: String,
    pub lineage_id: String,
    records: HashMap<String, ToolCallRecord>,
    #[serde(skip, default = "default_condvar")]
    condvar: Arc<Condvar>,
}

impl Default for ToolCallLedger {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            lineage_id: String::new(),
            records: HashMap::new(),
            condvar: default_condvar(),
        }
    }
}

impl ToolCallLedger {
    pub fn new(session_id: impl Into<String>, lineage_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            lineage_id: lineage_id.into(),
            records: HashMap::new(),
            condvar: default_condvar(),
        }
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
                    return Ok(ReservationOutcome::Replay {
                        output: rec.output.clone().unwrap_or_default(),
                        is_error: rec.is_error,
                    });
                }
                ToolExecutionStatus::Pending | ToolExecutionStatus::Executing => {
                    return Ok(ReservationOutcome::WaitForInFlight);
                }
            }
        }
        self.records.insert(
            call_id.to_string(),
            ToolCallRecord {
                call_id: call_id.to_string(),
                tool_name: tool_name.to_string(),
                normalized_arguments: norm_args,
                argument_digest: arg_digest,
                side_effect: classify_side_effect(tool_name),
                status: ToolExecutionStatus::Pending,
                result_digest: None,
                output: None,
                is_error: false,
                executed_at: None,
            },
        );
        Ok(ReservationOutcome::Reserved)
    }

    /// Mark an execution as beginning, or return Replay / WaitForInFlight / Collision.
    pub fn begin_execution(
        &mut self,
        call_id: &str,
        tool_name: &str,
        arguments: &Value,
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
                | ToolExecutionStatus::Blocked => BeginOutcome::Replay {
                    output: rec.output.clone().unwrap_or_default(),
                    is_error: rec.is_error,
                },
                ToolExecutionStatus::Executing => BeginOutcome::WaitForInFlight,
                ToolExecutionStatus::Pending => {
                    rec.status = ToolExecutionStatus::Executing;
                    BeginOutcome::Execute
                }
            }
        } else {
            self.records.insert(
                call_id.to_string(),
                ToolCallRecord {
                    call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    normalized_arguments: norm_args,
                    argument_digest: arg_digest,
                    side_effect: classify_side_effect(tool_name),
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

    pub fn cancel_reservation(&mut self, call_id: &str) {
        if let Some(rec) = self.records.get(call_id) {
            if rec.status == ToolExecutionStatus::Pending {
                self.records.remove(call_id);
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
            self.records.insert(
                call_id.to_string(),
                ToolCallRecord {
                    call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    normalized_arguments: norm_args,
                    argument_digest: arg_digest,
                    side_effect: classify_side_effect(tool_name),
                    status: ToolExecutionStatus::Executing,
                    result_digest: None,
                    output: None,
                    is_error: false,
                    executed_at: None,
                },
            );
        }
    }

    pub fn record_completion(&mut self, call_id: &str, output: &str, is_error: bool) {
        if let Some(entry) = self.records.get_mut(call_id) {
            entry.status = ToolExecutionStatus::Completed;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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

        // Matching args succeed with Replay
        let res = ledger
            .reserve_call(call_id, "bash", &json!({"command": "echo one"}))
            .unwrap();
        assert_eq!(
            res,
            ReservationOutcome::Replay {
                output: "one\n".into(),
                is_error: false
            }
        );
    }

    #[test]
    fn canonical_arguments_order_independent() {
        let mut ledger = ToolCallLedger::new("sess_1", "lin_1");
        let call_id = "call_canon";
        let args1 = json!({"a": 1, "b": 2});
        let args2 = json!({"b": 2, "a": 1});
        ledger.record_start(call_id, "my_tool", &args1);
        ledger.record_completion(call_id, "done", false);

        let res = ledger.reserve_call(call_id, "my_tool", &args2).unwrap();
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
}
