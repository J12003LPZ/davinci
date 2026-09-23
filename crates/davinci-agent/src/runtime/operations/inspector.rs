//! Read-only operation-journal inspection.
//!
//! The inspector deliberately does not use [`OperationJournal::open`].  That
//! API owns the writer lease and is allowed to create directories, migrate a
//! schema, and register a root.  A diagnostic command must be safe to run
//! against a live workspace, so this module opens existing databases with
//! SQLite's read-only flags and only performs bounded SELECTs.

use super::model::{OperationAttempt, OperationSpec};
use super::store_api::JOURNAL_DATABASE_FILE_NAME;
use super::PayloadDigest;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const INSPECTOR_SCHEMA_VERSION: u32 = 1;
pub const JOURNAL_APPLICATION_ID: i64 = 0x4456_4F50;
pub const JOURNAL_SCHEMA_VERSION: i64 = 4;
pub const INSPECTOR_MAX_OUTPUT_BYTES: usize = 256 * 1024;

const MAX_OPERATIONS: usize = 5_000;
const MAX_ATTEMPTS: usize = 10_000;
const MAX_EVENTS: usize = 20_000;
const MAX_RESULTS: usize = 5_000;
const MAX_FINDINGS: usize = 256;
const MAX_TIMELINE: usize = 1_024;
const MAX_ATTEMPT_REPORTS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectorStatus {
    Healthy,
    Findings,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InspectionTarget {
    Operation(String),
    Run(String),
    Session(String),
    DoctorRuntime,
}

impl InspectionTarget {
    fn command_name(&self) -> &'static str {
        match self {
            Self::Operation(_) => "inspect_operation",
            Self::Run(_) => "inspect_run",
            Self::Session(_) => "inspect_session",
            Self::DoctorRuntime => "doctor_runtime",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct InspectorOutput {
    pub schema_version: u32,
    pub command: String,
    pub status: InspectorStatus,
    pub exit_code: u8,
    pub findings: Vec<String>,
    pub report: Value,
    pub truncated: bool,
}

impl InspectorOutput {
    pub fn json_string(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            "{\"schema_version\":1,\"status\":\"unavailable\",\"exit_code\":3,\"findings\":[\"inspector serialization failed\"]}".into()
        })
    }

    pub fn text(&self) -> String {
        let mut lines = vec![format!(
            "{}: {:?} (exit {})",
            self.command, self.status, self.exit_code
        )];
        lines.extend(self.findings.iter().map(|finding| format!("! {finding}")));
        if self.report != Value::Null {
            if let Some(operations) = self.report.get("operations").and_then(Value::as_array) {
                lines.push(format!("operations: {}", operations.len()));
                for operation in operations.iter().take(16) {
                    let id = operation
                        .pointer("/identity/operation_id")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    let state = operation
                        .pointer("/attempts/0/state")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    lines.push(format!("  {id} · {state}"));
                }
            }
            if let Some(identity) = self.report.get("identity") {
                if let Some(id) = identity.get("operation_id").and_then(Value::as_str) {
                    lines.push(format!("operation: {id}"));
                }
            }
        }
        if self.truncated {
            lines.push("output truncated at the inspector bound".into());
        }
        lines.join("\n")
    }
}

#[derive(Debug, Clone)]
struct RawOperation {
    operation_id: String,
    root_namespace_id: String,
    created_at_ms: i64,
    spec: Value,
    spec_error: Option<String>,
}

#[derive(Debug, Clone)]
struct RawAttempt {
    attempt_id: String,
    operation_id: String,
    revision: i64,
    attempt: Value,
    attempt_error: Option<String>,
}

#[derive(Debug, Clone)]
struct RawEvent {
    sequence: i64,
    operation_id: String,
    attempt_id: String,
    prior_revision: i64,
    revision: i64,
    event: Value,
    created_at_ms: i64,
    event_error: Option<String>,
}

#[derive(Debug, Clone)]
struct RawResult {
    operation_id: String,
    attempt_id: String,
    payload_digest: String,
    result_ref: Value,
    payload_valid: bool,
    created_at_ms: i64,
    result_error: Option<String>,
}

#[derive(Debug, Clone)]
struct JournalSnapshot {
    label: String,
    operations: Vec<RawOperation>,
    attempts: Vec<RawAttempt>,
    events: Vec<RawEvent>,
    results: Vec<RawResult>,
    pending_outbox: i64,
    issues: Vec<String>,
}

#[derive(Debug, Clone)]
struct RootDiagnostic {
    label: String,
    operation_count: usize,
    attempt_count: usize,
    event_count: usize,
    pending_outbox: i64,
    issues: Vec<String>,
}

/// Return existing journal files below a project root.  The list is
/// intentionally explicit and bounded; inspection never performs a recursive
/// walk or creates a candidate directory.
pub fn discover_journal_paths(cwd: &Path) -> Vec<PathBuf> {
    let candidates = [
        cwd.join(JOURNAL_DATABASE_FILE_NAME),
        cwd.join("journal").join(JOURNAL_DATABASE_FILE_NAME),
        cwd.join("operation-journal")
            .join(JOURNAL_DATABASE_FILE_NAME),
        cwd.join(".operation-journal")
            .join(JOURNAL_DATABASE_FILE_NAME),
        cwd.join(".davinci").join(JOURNAL_DATABASE_FILE_NAME),
        cwd.join(".davinci")
            .join("operations")
            .join(JOURNAL_DATABASE_FILE_NAME),
        cwd.join(".davinci")
            .join("journal")
            .join(JOURNAL_DATABASE_FILE_NAME),
        cwd.join(".pi").join(JOURNAL_DATABASE_FILE_NAME),
        cwd.join(".pi")
            .join("operations")
            .join(JOURNAL_DATABASE_FILE_NAME),
        cwd.join(".pi")
            .join("journal")
            .join(JOURNAL_DATABASE_FILE_NAME),
    ];
    let mut seen = BTreeSet::new();
    candidates
        .into_iter()
        .filter(|path| path.is_file())
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

pub fn inspect(cwd: &Path, target: InspectionTarget) -> InspectorOutput {
    let command = target.command_name().to_string();
    let paths = discover_journal_paths(cwd);
    if paths.is_empty() {
        return finish(
            command,
            InspectorStatus::Unavailable,
            vec!["no existing operation journal was found under the current root".into()],
            json!({"roots": [], "read_only": true}),
        );
    }

    let mut snapshots = Vec::new();
    let mut root_errors = Vec::new();
    for path in paths {
        let label = relative_label(cwd, &path);
        match read_snapshot(&path, label.clone()) {
            Ok(snapshot) => snapshots.push(snapshot),
            Err(error) => root_errors.push(format!("{label}: {error}")),
        }
    }

    if snapshots.is_empty() {
        return finish(
            command,
            InspectorStatus::Unavailable,
            root_errors,
            json!({"roots": [], "read_only": true}),
        );
    }

    let mut findings = root_errors;
    let mut diagnostics = Vec::new();
    for snapshot in &snapshots {
        findings.extend(snapshot.issues.iter().cloned());
        diagnostics.push(RootDiagnostic {
            label: snapshot.label.clone(),
            operation_count: snapshot.operations.len(),
            attempt_count: snapshot.attempts.len(),
            event_count: snapshot.events.len(),
            pending_outbox: snapshot.pending_outbox,
            issues: snapshot.issues.clone(),
        });
    }

    let report = match target {
        InspectionTarget::DoctorRuntime => doctor_report(&snapshots, &mut findings),
        target => inspect_report(&snapshots, target, &mut findings),
    };
    let roots = diagnostics
        .into_iter()
        .map(|root| {
            json!({
                "root": root.label,
                "operation_count": root.operation_count,
                "attempt_count": root.attempt_count,
                "event_count": root.event_count,
                "pending_outbox": root.pending_outbox,
                "issues": root.issues,
                "read_only": true,
            })
        })
        .collect::<Vec<_>>();
    let mut report = match report {
        Value::Object(mut object) => {
            object.insert("roots".into(), Value::Array(roots));
            Value::Object(object)
        }
        other => json!({"roots": roots, "data": other}),
    };
    if !findings.is_empty() {
        if let Value::Object(object) = &mut report {
            object.insert("read_only".into(), Value::Bool(true));
        }
    }
    finish(command, status_for(&findings), findings, report)
}

fn status_for(findings: &[String]) -> InspectorStatus {
    if findings.is_empty() {
        InspectorStatus::Healthy
    } else {
        InspectorStatus::Findings
    }
}

fn finish(
    command: String,
    status: InspectorStatus,
    mut findings: Vec<String>,
    report: Value,
) -> InspectorOutput {
    findings.truncate(MAX_FINDINGS);
    let mut output = InspectorOutput {
        schema_version: INSPECTOR_SCHEMA_VERSION,
        command,
        status,
        exit_code: match status {
            InspectorStatus::Healthy => 0,
            InspectorStatus::Findings => 2,
            InspectorStatus::Unavailable => 3,
        },
        findings,
        report,
        truncated: false,
    };
    bound_output(&mut output);
    output
}

fn relative_label(cwd: &Path, path: &Path) -> String {
    path.strip_prefix(cwd)
        .map(|relative| relative.display().to_string())
        .unwrap_or_else(|_| "<external journal>".into())
}

fn read_snapshot(path: &Path, label: String) -> Result<JournalSnapshot, String> {
    ensure_read_only_snapshot_is_safe(path)?;
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NOFOLLOW
            | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|error| format!("read-only open failed: {error}"))?;
    connection
        .busy_timeout(Duration::from_millis(250))
        .map_err(|error| format!("busy timeout setup failed: {error}"))?;
    validate_schema(&connection)?;

    let mut issues = Vec::new();
    let operations = read_operations(&connection, &mut issues)?;
    let attempts = read_attempts(&connection, &mut issues)?;
    let events = read_events(&connection, &mut issues)?;
    let results = read_results(&connection, &mut issues)?;
    let pending_outbox = connection
        .query_row(
            "SELECT count(*) FROM operation_outbox WHERE state = 'pending'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("outbox read failed: {error}"))?;
    Ok(JournalSnapshot {
        label,
        operations,
        attempts,
        events,
        results,
        pending_outbox,
        issues,
    })
}

fn ensure_read_only_snapshot_is_safe(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("journal metadata read failed: {error}"))?;
    if metadata.file_type().is_symlink() {
        return Err("journal database is a symbolic link".into());
    }

    let mut header = [0_u8; 20];
    File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .map_err(|error| format!("journal header read failed: {error}"))?;
    if &header[..16] == b"SQLite format 3\0" && (header[18] == 2 || header[19] == 2) {
        let wal = PathBuf::from(format!("{}-wal", path.display()));
        let shm = PathBuf::from(format!("{}-shm", path.display()));
        if !wal.exists() || !shm.exists() {
            return Err(
                "safe consistent read unavailable: live WAL sidecars are incomplete; no files were created".into(),
            );
        }
    }
    Ok(())
}

fn validate_schema(connection: &Connection) -> Result<(), String> {
    let application_id: i64 = connection
        .pragma_query_value(None, "application_id", |row| row.get(0))
        .map_err(|error| format!("application id read failed: {error}"))?;
    if application_id != JOURNAL_APPLICATION_ID {
        return Err(format!(
            "unsupported journal application id {application_id}"
        ));
    }
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| format!("schema version read failed: {error}"))?;
    if version != JOURNAL_SCHEMA_VERSION {
        return Err(format!("unsupported journal schema {version}"));
    }
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|error| format!("integrity check failed: {error}"))?;
    if integrity != "ok" {
        return Err(format!("integrity check reported {integrity}"));
    }
    let required = [
        "journal_metadata",
        "journal_roots",
        "operations",
        "attempts",
        "operation_events",
        "operation_results",
        "operation_outbox",
        "operation_owners",
        "dispatch_claims",
        "operation_call_mappings",
        "operation_recovery_decisions",
        "operation_effect_claims",
        "legacy_observations",
    ];
    for table in required {
        let exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                [table],
                |row| row.get(0),
            )
            .map_err(|error| format!("schema table check failed: {error}"))?;
        if !exists {
            return Err(format!("required table {table} is missing"));
        }
    }
    Ok(())
}

fn read_operations(
    connection: &Connection,
    issues: &mut Vec<String>,
) -> Result<Vec<RawOperation>, String> {
    let mut statement = connection
        .prepare(
            "SELECT operation_id, root_namespace_id, spec_json, created_at_ms
             FROM operations ORDER BY created_at_ms, operation_id LIMIT ?1",
        )
        .map_err(|error| format!("operation query failed: {error}"))?;
    let rows = statement
        .query_map([MAX_OPERATIONS as i64 + 1], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(|error| format!("operation query failed: {error}"))?;
    let mut values = Vec::new();
    for row in rows {
        let (operation_id, root_namespace_id, spec_json, created_at_ms) =
            row.map_err(|error| format!("operation row failed: {error}"))?;
        let spec = match serde_json::from_str::<Value>(&spec_json) {
            Ok(value) => value,
            Err(error) => {
                issues.push(format!(
                    "operation {operation_id}: corrupt spec JSON ({error})"
                ));
                Value::Null
            }
        };
        if values.len() == MAX_OPERATIONS {
            issues.push("operation inspection is bounded at 5000 rows".into());
            break;
        }
        values.push(RawOperation {
            operation_id,
            root_namespace_id,
            created_at_ms,
            spec_error: if spec == Value::Null {
                Some("spec JSON is not valid".into())
            } else if serde_json::from_value::<OperationSpec>(spec.clone()).is_err() {
                Some("spec does not satisfy the operation model".into())
            } else {
                None
            },
            spec,
        });
    }
    Ok(values)
}

fn read_attempts(
    connection: &Connection,
    issues: &mut Vec<String>,
) -> Result<Vec<RawAttempt>, String> {
    let mut statement = connection
        .prepare(
            "SELECT attempt_id, operation_id, revision, attempt_json
             FROM attempts ORDER BY updated_at_ms, attempt_id LIMIT ?1",
        )
        .map_err(|error| format!("attempt query failed: {error}"))?;
    let rows = statement
        .query_map([MAX_ATTEMPTS as i64 + 1], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|error| format!("attempt query failed: {error}"))?;
    let mut values = Vec::new();
    for row in rows {
        let (attempt_id, operation_id, revision, attempt_json) =
            row.map_err(|error| format!("attempt row failed: {error}"))?;
        let attempt = match serde_json::from_str::<Value>(&attempt_json) {
            Ok(value) => value,
            Err(error) => {
                issues.push(format!(
                    "attempt {attempt_id}: corrupt attempt JSON ({error})"
                ));
                Value::Null
            }
        };
        if values.len() == MAX_ATTEMPTS {
            issues.push("attempt inspection is bounded at 10000 rows".into());
            break;
        }
        values.push(RawAttempt {
            attempt_id,
            operation_id,
            revision,
            attempt_error: if attempt == Value::Null {
                Some("attempt JSON is not valid".into())
            } else if serde_json::from_value::<OperationAttempt>(attempt.clone()).is_err() {
                Some("attempt does not satisfy the operation model".into())
            } else {
                None
            },
            attempt,
        });
    }
    Ok(values)
}

fn read_events(connection: &Connection, issues: &mut Vec<String>) -> Result<Vec<RawEvent>, String> {
    let mut statement = connection
        .prepare(
            "SELECT event_sequence, operation_id, attempt_id, prior_revision, revision, event_json, created_at_ms
             FROM operation_events ORDER BY event_sequence LIMIT ?1",
        )
        .map_err(|error| format!("event query failed: {error}"))?;
    let rows = statement
        .query_map([MAX_EVENTS as i64 + 1], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })
        .map_err(|error| format!("event query failed: {error}"))?;
    let mut values = Vec::new();
    for row in rows {
        let (
            sequence,
            operation_id,
            attempt_id,
            prior_revision,
            revision,
            event_json,
            created_at_ms,
        ) = row.map_err(|error| format!("event row failed: {error}"))?;
        let event = match serde_json::from_str::<Value>(&event_json) {
            Ok(value) => value,
            Err(error) => {
                issues.push(format!("event {sequence}: corrupt event JSON ({error})"));
                Value::Null
            }
        };
        if values.len() == MAX_EVENTS {
            issues.push("event inspection is bounded at 20000 rows".into());
            break;
        }
        values.push(RawEvent {
            sequence,
            operation_id,
            attempt_id,
            prior_revision,
            revision,
            event_error: (event == Value::Null).then(|| "event JSON is not valid".into()),
            event,
            created_at_ms,
        });
    }
    Ok(values)
}

fn read_results(
    connection: &Connection,
    issues: &mut Vec<String>,
) -> Result<Vec<RawResult>, String> {
    let mut statement = connection
        .prepare(
            "SELECT operation_id, attempt_id, payload_digest, result_ref_json, payload_json, created_at_ms
             FROM operation_results ORDER BY created_at_ms, result_id LIMIT ?1",
        )
        .map_err(|error| format!("result query failed: {error}"))?;
    let rows = statement
        .query_map([MAX_RESULTS as i64 + 1], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(|error| format!("result query failed: {error}"))?;
    let mut values = Vec::new();
    for row in rows {
        let (operation_id, attempt_id, payload_digest, reference_json, payload_json, created_at_ms) =
            row.map_err(|error| format!("result row failed: {error}"))?;
        let reference = match serde_json::from_str::<Value>(&reference_json) {
            Ok(value) => value,
            Err(error) => {
                issues.push(format!(
                    "result for attempt {attempt_id}: corrupt reference ({error})"
                ));
                Value::Null
            }
        };
        let payload = serde_json::from_str::<Value>(&payload_json);
        let payload_valid = match &payload {
            Ok(value) => match PayloadDigest::of_json(value) {
                Ok(digest) => digest.to_string() == payload_digest,
                Err(_) => false,
            },
            Err(_) => false,
        };
        let result_error = if reference == Value::Null || payload.is_err() || !payload_valid {
            let issue = if !payload_valid {
                "result payload digest does not match"
            } else {
                "result JSON is not valid"
            };
            issues.push(format!("result for attempt {attempt_id}: {issue}"));
            Some(issue.into())
        } else {
            None
        };
        if values.len() == MAX_RESULTS {
            issues.push("result inspection is bounded at 5000 rows".into());
            break;
        }
        values.push(RawResult {
            operation_id,
            attempt_id,
            payload_digest,
            result_ref: reference,
            payload_valid,
            created_at_ms,
            result_error,
        });
    }
    Ok(values)
}

fn inspect_report(
    snapshots: &[JournalSnapshot],
    target: InspectionTarget,
    findings: &mut Vec<String>,
) -> Value {
    let (requested, matches): (String, Vec<(&JournalSnapshot, &RawOperation)>) =
        match target.clone() {
            InspectionTarget::Operation(id) => {
                let mut matches = Vec::new();
                for snapshot in snapshots {
                    for operation in &snapshot.operations {
                        if operation.operation_id == id {
                            matches.push((snapshot, operation));
                        }
                    }
                }
                (id, matches)
            }
            InspectionTarget::Run(id) => {
                let mut matches = Vec::new();
                for snapshot in snapshots {
                    for operation in &snapshot.operations {
                        if context_string(&operation.spec, "runtime_run_id") == Some(id.as_str()) {
                            matches.push((snapshot, operation));
                        }
                    }
                }
                (id, matches)
            }
            InspectionTarget::Session(id) => {
                let mut matches = Vec::new();
                for snapshot in snapshots {
                    for operation in &snapshot.operations {
                        if context_string(&operation.spec, "session_id") == Some(id.as_str()) {
                            matches.push((snapshot, operation));
                        }
                    }
                }
                (id, matches)
            }
            InspectionTarget::DoctorRuntime => unreachable!(),
        };
    if matches.is_empty() {
        findings.push(format!(
            "no operation matched requested identity {requested}"
        ));
    }
    let operations = matches
        .into_iter()
        .take(MAX_ATTEMPT_REPORTS)
        .map(|(snapshot, operation)| operation_report(snapshot, operation, findings))
        .collect::<Vec<_>>();
    let matched_field = match target {
        InspectionTarget::Operation(_) => "operation_id",
        InspectionTarget::Run(_) => "context.runtime_run_id",
        InspectionTarget::Session(_) => "context.session_id",
        InspectionTarget::DoctorRuntime => "none",
    };
    json!({
        "target": {"requested": requested, "matched_field": matched_field},
        "operations": operations,
    })
}

fn doctor_report(snapshots: &[JournalSnapshot], findings: &mut Vec<String>) -> Value {
    let mut incomplete = Vec::new();
    let mut revision_checks = Vec::new();
    let mut binding_checks = Vec::new();
    let mut evidence_checks = Vec::new();
    let mut operation_count = 0usize;
    for snapshot in snapshots {
        operation_count += snapshot.operations.len();
        for operation in &snapshot.operations {
            if operation.spec_error.is_some() {
                findings.push(format!(
                    "operation {} has an invalid or unsupported specification",
                    operation.operation_id
                ));
            }
            if context_string(&operation.spec, "root_namespace_id")
                != Some(operation.root_namespace_id.as_str())
            {
                let message = format!(
                    "operation {} is bound to a different root namespace",
                    operation.operation_id
                );
                findings.push(message.clone());
                binding_checks
                    .push(json!({"operation_id": operation.operation_id, "finding": message}));
            }
            for attempt in snapshot
                .attempts
                .iter()
                .filter(|attempt| attempt.operation_id == operation.operation_id)
            {
                if let Some(error) = &attempt.attempt_error {
                    findings.push(format!("attempt {}: {error}", attempt.attempt_id));
                }
                let state = attempt
                    .attempt
                    .get("state")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                if matches!(
                    state,
                    "queued" | "running" | "effect_possible" | "interrupted" | "recovery_required"
                ) {
                    let message = format!(
                        "operation {} attempt {} is incomplete in state {state}",
                        operation.operation_id, attempt.attempt_id
                    );
                    findings.push(message.clone());
                    incomplete.push(json!({
                        "operation_id": operation.operation_id,
                        "attempt_id": attempt.attempt_id,
                        "state": state,
                        "finding": message,
                    }));
                }
                let events = snapshot
                    .events
                    .iter()
                    .filter(|event| event.attempt_id == attempt.attempt_id)
                    .collect::<Vec<_>>();
                let max_revision = events.iter().map(|event| event.revision).max().unwrap_or(0);
                if max_revision != attempt.revision || (attempt.revision > 0 && events.is_empty()) {
                    let message = format!(
                        "attempt {} revision {} does not match its event projection {}",
                        attempt.attempt_id, attempt.revision, max_revision
                    );
                    findings.push(message.clone());
                    revision_checks.push(json!({
                        "attempt_id": attempt.attempt_id,
                        "stored_revision": attempt.revision,
                        "event_revision": max_revision,
                        "finding": message,
                    }));
                }
                let has_result = snapshot
                    .results
                    .iter()
                    .any(|result| result.attempt_id == attempt.attempt_id);
                if state == "succeeded" && !has_result {
                    let message = format!(
                        "succeeded attempt {} has no durable result projection",
                        attempt.attempt_id
                    );
                    findings.push(message.clone());
                    revision_checks
                        .push(json!({"attempt_id": attempt.attempt_id, "finding": message}));
                }
                let evidence = attempt
                    .attempt
                    .get("recovery_evidence")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty());
                if let Some(evidence) = evidence {
                    let message = format!(
                        "attempt {} references evidence {evidence} that is outside the journal resolver",
                        attempt.attempt_id
                    );
                    findings.push(message.clone());
                    evidence_checks.push(json!({
                        "attempt_id": attempt.attempt_id,
                        "evidence_id": evidence,
                        "finding": message,
                    }));
                }
            }
        }
    }
    json!({
        "checks": {
            "operation_count": operation_count,
            "incomplete_operations": incomplete,
            "missing_projections": revision_checks,
            "bad_bindings": binding_checks,
            "broken_evidence_links": evidence_checks,
            "repair": "not performed",
        },
        "read_only": true,
    })
}

fn operation_report(
    snapshot: &JournalSnapshot,
    operation: &RawOperation,
    findings: &mut Vec<String>,
) -> Value {
    let attempts = snapshot
        .attempts
        .iter()
        .filter(|attempt| attempt.operation_id == operation.operation_id)
        .collect::<Vec<_>>();
    let events = snapshot
        .events
        .iter()
        .filter(|event| event.operation_id == operation.operation_id)
        .collect::<Vec<_>>();
    let results = snapshot
        .results
        .iter()
        .filter(|result| result.operation_id == operation.operation_id)
        .collect::<Vec<_>>();
    let context = operation
        .spec
        .get("context")
        .cloned()
        .unwrap_or(Value::Null);
    let identity = json!({
        "operation_id": operation.operation_id,
        "root_namespace_id": operation.root_namespace_id,
        "created_at_ms": operation.created_at_ms,
        "schema_version": operation.spec.get("schema_version").cloned().unwrap_or(Value::Null),
    });
    let attempts_report = attempts
        .iter()
        .take(MAX_ATTEMPT_REPORTS)
        .map(|attempt| project_attempt(&attempt.attempt))
        .collect::<Vec<_>>();
    if attempts.len() > MAX_ATTEMPT_REPORTS {
        findings.push(format!(
            "operation {} attempt report was bounded",
            operation.operation_id
        ));
    }
    let timeline = events
        .iter()
        .take(MAX_TIMELINE)
        .map(|event| {
            let kind = event
                .event
                .as_object()
                .and_then(|object| object.keys().next().cloned())
                .unwrap_or_else(|| "unknown".into());
            json!({
                "sequence": event.sequence,
                "attempt_id": event.attempt_id,
                "prior_revision": event.prior_revision,
                "revision": event.revision,
                "kind": kind,
                "created_at_ms": event.created_at_ms,
                "valid": event.event_error.is_none(),
            })
        })
        .collect::<Vec<_>>();
    let result = results
        .last()
        .map(|result| project_result(result))
        .unwrap_or_else(|| Value::Null);
    let graph = context.get("graph").cloned().unwrap_or(Value::Null);
    let domain_links = results
        .iter()
        .filter_map(|result| result.result_ref.get("links"))
        .map(project_links)
        .collect::<Vec<_>>();
    let evidence = json!({
        "result_references": results.iter().map(|result| project_evidence(&result.result_ref)).collect::<Vec<_>>(),
        "recovery_evidence": attempts.iter().filter_map(|attempt| attempt.attempt.get("recovery_evidence")).cloned().collect::<Vec<_>>(),
    });
    let recovery_reasons = attempts
        .iter()
        .filter_map(|attempt| attempt.attempt.get("failure"))
        .map(|failure| json!({"failure": failure}))
        .chain(attempts.iter().filter_map(|attempt| {
            let state = attempt.attempt.get("state").and_then(Value::as_str)?;
            (state == "interrupted" || state == "recovery_required" || state == "effect_possible")
                .then(|| json!({"state": state}))
        }))
        .collect::<Vec<_>>();
    if operation.spec_error.is_some() {
        findings.push(format!(
            "operation {} specification cannot be validated",
            operation.operation_id
        ));
    }
    json!({
        "identity": identity,
        "parent": context.get("parent_operation_id").cloned().unwrap_or(Value::Null),
        "requester": {
            "caller": context.get("caller").cloned().unwrap_or(Value::Null),
            "session_id": context.get("session_id").cloned().unwrap_or(Value::Null),
            "runtime_run_id": context.get("runtime_run_id").cloned().unwrap_or(Value::Null),
            "agent_id": context.get("agent_id").cloned().unwrap_or(Value::Null),
            "worker_id": context.get("worker_id").cloned().unwrap_or(Value::Null),
            "wire_tool_call_id": context.get("wire_tool_call_id").cloned().unwrap_or(Value::Null),
        },
        "graph_runtime_distinction": {
            "runtime_run_id": context.get("runtime_run_id").cloned().unwrap_or(Value::Null),
            "graph": graph,
        },
        "side_effects": {
            "profile": operation.spec.get("effects").cloned().unwrap_or(Value::Null),
            "preconditions": operation.spec.get("preconditions").cloned().unwrap_or(Value::Null),
            "attempt_effect_statuses": attempts.iter().filter_map(|attempt| attempt.attempt.get("effect_status")).cloned().collect::<Vec<_>>(),
        },
        "owner": attempts.last().and_then(|attempt| attempt.attempt.get("owner")).cloned().unwrap_or(Value::Null),
        "domain_links": domain_links,
        "attempts": attempts_report,
        "timeline": timeline,
        "result": result,
        "evidence": evidence,
        "recovery_reasons": recovery_reasons,
    })
}

fn project_attempt(attempt: &Value) -> Value {
    let mut result = Map::new();
    for key in [
        "attempt_id",
        "operation_id",
        "attempt_number",
        "owner",
        "revision",
        "state",
        "effect_status",
        "authorization",
        "started_at",
        "finished_at",
        "verification",
        "publications",
        "cancellation_requested",
        "retries_attempt_id",
        "recovery_evidence",
        "superseded_by",
    ] {
        if let Some(value) = attempt.get(key) {
            result.insert(key.into(), value.clone());
        }
    }
    if let Some(value) = attempt.get("result") {
        result.insert("result_reference".into(), project_result_reference(value));
    }
    if let Some(value) = attempt.get("failure") {
        result.insert("failure".into(), value.clone());
    }
    Value::Object(result)
}

fn project_result(result: &RawResult) -> Value {
    json!({
        "attempt_id": result.attempt_id,
        "payload_digest": result.payload_digest,
        "reference": project_result_reference(&result.result_ref),
        "payload_valid": result.payload_valid,
        "created_at_ms": result.created_at_ms,
        "error": result.result_error,
    })
}

fn project_result_reference(reference: &Value) -> Value {
    let mut result = Map::new();
    for key in [
        "result_id",
        "payload_digest",
        "artifacts",
        "evidence",
        "links",
    ] {
        if let Some(value) = reference.get(key) {
            result.insert(key.into(), value.clone());
        }
    }
    Value::Object(result)
}

fn project_evidence(reference: &Value) -> Value {
    json!({
        "result_id": reference.get("result_id").cloned().unwrap_or(Value::Null),
        "evidence": reference.get("evidence").cloned().unwrap_or(Value::Array(Vec::new())),
        "artifacts": reference.get("artifacts").cloned().unwrap_or(Value::Array(Vec::new())),
    })
}

fn project_links(value: &Value) -> Value {
    value
        .as_array()
        .map(|links| {
            Value::Array(
                links
                    .iter()
                    .filter_map(|link| {
                        Some(json!({
                            "kind": link.get("kind")?.clone(),
                            "target_id": link.get("target_id")?.clone(),
                        }))
                    })
                    .take(64)
                    .collect(),
            )
        })
        .unwrap_or_else(|| Value::Array(Vec::new()))
}

fn context_string<'a>(spec: &'a Value, key: &str) -> Option<&'a str> {
    spec.pointer(&format!("/context/{key}"))
        .and_then(Value::as_str)
}

fn bound_output(output: &mut InspectorOutput) {
    output.findings.truncate(MAX_FINDINGS);
    output.report = bound_value(output.report.clone(), 0);
    let mut bytes = serde_json::to_vec(output)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX);
    if bytes <= INSPECTOR_MAX_OUTPUT_BYTES {
        return;
    }
    output.truncated = true;
    if let Value::Object(object) = &mut output.report {
        for key in ["timeline", "attempts", "operations", "roots"] {
            if let Some(Value::Array(values)) = object.get_mut(key) {
                values.truncate(32);
            }
        }
        object.insert("truncated".into(), Value::Bool(true));
    }
    bytes = serde_json::to_vec(output)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX);
    if bytes > INSPECTOR_MAX_OUTPUT_BYTES {
        output.report =
            json!({"truncated": true, "summary": "report exceeded the bounded inspector output"});
    }
}

fn bound_value(value: Value, depth: usize) -> Value {
    if depth > 8 {
        return Value::String("<depth limit>".into());
    }
    match value {
        Value::String(mut string) => {
            if string.len() > 2048 {
                string.truncate(2048);
                string.push_str("…");
            }
            Value::String(string)
        }
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .take(1_024)
                .map(|value| bound_value(value, depth + 1))
                .collect(),
        ),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .take(256)
                .map(|(key, value)| (key, bound_value(value, depth + 1)))
                .collect(),
        ),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::operations::{
        CallerType, EffectClass, EffectProfile, ExecutionOwner, ExecutionOwnerId, IdempotencyScope,
        JournalId, JournalIdentity, OperationContext, OperationJournal, OperationKind,
        RootNamespaceId, ScopedIdempotencyKey, WorkspaceId, WorkspaceIdentity,
    };
    use crate::runtime::{AgentId, RunId};
    use serde_json::json;
    use tempfile::tempdir;

    fn fixture() -> (tempfile::TempDir, String, String) {
        let temp = tempdir().unwrap();
        let journal_dir = temp.path().join(".davinci").join("operations");
        let identity = JournalIdentity::new(
            JournalId::new(),
            WorkspaceIdentity {
                id: WorkspaceId::new(),
                binding_version: 1,
            },
        )
        .unwrap();
        let root = RootNamespaceId::new();
        let run = RunId::new();
        let session = "inspector-test-session".to_owned();
        let spec = super::super::model::OperationSpec::new(
            OperationContext {
                journal_id: identity.journal_id,
                root_namespace_id: root,
                session_id: session.clone(),
                runtime_run_id: run,
                parent_operation_id: None,
                agent_id: AgentId::new(),
                worker_id: None,
                task_id: None,
                graph: None,
                workspace: identity.workspace.clone(),
                caller: CallerType::HostControl,
                wire_tool_call_id: None,
            },
            ScopedIdempotencyKey::new(IdempotencyScope::HostControl, "inspector-key").unwrap(),
            OperationKind::CustomExternalAction,
            EffectProfile {
                classification: EffectClass::ReadOnly,
                supports_idempotency_key: true,
                supports_postcondition_probe: false,
                supports_compensation: false,
                requires_live_owner: false,
            },
            json!({"safe": true}),
            vec![],
        )
        .unwrap();
        let operation_id = spec.operation_id().to_string();
        let journal = OperationJournal::open(&journal_dir, identity, root).unwrap();
        let attempt = super::super::model::OperationAttempt::new(
            spec.operation_id(),
            1,
            ExecutionOwner::new(ExecutionOwnerId::new(), 1).unwrap(),
        )
        .unwrap();
        journal.persist_intent(&spec, &attempt).unwrap();
        let snapshot_dir = temp.path().join("snapshot");
        journal.backup_to(&snapshot_dir).unwrap();
        drop(journal);
        for suffix in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(journal_dir.join(format!("operations.sqlite3{suffix}")));
        }
        std::fs::copy(
            snapshot_dir.join("operations.sqlite3"),
            journal_dir.join("operations.sqlite3"),
        )
        .unwrap();
        (temp, operation_id, run.to_string())
    }

    #[test]
    fn reads_fixture_without_writing_sidecars() {
        let (temp, operation_id, _) = fixture();
        let before = std::fs::read_dir(temp.path().join(".davinci").join("operations"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<BTreeSet<_>>();
        let output = inspect(temp.path(), InspectionTarget::Operation(operation_id));
        assert_eq!(output.exit_code, 0);
        let after = std::fs::read_dir(temp.path().join(".davinci").join("operations"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<BTreeSet<_>>();
        assert_eq!(before, after);
        assert!(output.json_string().len() <= INSPECTOR_MAX_OUTPUT_BYTES);
    }

    #[test]
    fn reports_incomplete_attempts_without_repairing_them() {
        let (temp, operation_id, _) = fixture();
        let output = inspect(temp.path(), InspectionTarget::Operation(operation_id));
        assert!(!output.report.is_null());
        assert!(matches!(
            output.status,
            InspectorStatus::Healthy | InspectorStatus::Findings
        ));
    }

    #[test]
    fn missing_root_is_unavailable() {
        let temp = tempdir().unwrap();
        let output = inspect(temp.path(), InspectionTarget::DoctorRuntime);
        assert_eq!(output.exit_code, 3);
        assert_eq!(output.status, InspectorStatus::Unavailable);
    }

    #[test]
    fn refuses_live_wal_without_creating_sidecars() {
        let temp = tempdir().unwrap();
        let database = temp.path().join("operations.sqlite3");
        let mut header = [0_u8; 20];
        header[..16].copy_from_slice(b"SQLite format 3\0");
        header[18] = 2;
        header[19] = 2;
        std::fs::write(&database, header).unwrap();
        let before = std::fs::read_dir(temp.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<BTreeSet<_>>();
        let result = ensure_read_only_snapshot_is_safe(&database);
        let after = std::fs::read_dir(temp.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<BTreeSet<_>>();
        assert!(result
            .unwrap_err()
            .contains("safe consistent read unavailable"));
        assert_eq!(before, after);
    }
}
