//! Versioned JSONL sidecar log for runtime lifecycle events (`<session>.runtime.jsonl`).
//!
//! Stores `RuntimeEventEnvelope` rows alongside session JSONL files without modifying
//! the upstream session format.

use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const CURRENT_RUNTIME_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Error)]
pub enum RuntimeLogError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Unsupported runtime log schema version: found {found}, max supported is {supported}")]
    UnsupportedSchemaVersion { found: u16, supported: u16 },
    #[error("Corrupt record on line {line}: {reason}")]
    CorruptRecord { line: usize, reason: String },
}

/// Derive the sidecar runtime log path for a given session file path.
/// E.g. `/path/to/2026-09-05_session-123.jsonl` -> `/path/to/2026-09-05_session-123.runtime.jsonl`.
pub fn runtime_log_path(session_path: &Path) -> PathBuf {
    let file_name = session_path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or_default();
    if let Some(prefix) = file_name.strip_suffix(".jsonl") {
        session_path.with_file_name(format!("{prefix}.runtime.jsonl"))
    } else {
        session_path.with_extension("runtime.jsonl")
    }
}

/// Appender for runtime event sidecar logs.
#[derive(Debug)]
pub struct RuntimeLogWriter {
    path: PathBuf,
    file: File,
}

impl RuntimeLogWriter {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RuntimeLogError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            davinci_sys::fs::create_private_dir_all(parent)?;
        }
        davinci_sys::fs::truncate_torn_tail(&path)?;
        let file = davinci_sys::fs::open_append_private(&path)?;
        Ok(Self { path, file })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append<T: serde::Serialize>(&mut self, record: &T) -> Result<(), RuntimeLogError> {
        let line = serde_json::to_string(record)?;
        writeln!(self.file, "{line}")?;
        self.file.sync_data()?;
        Ok(())
    }
}

/// Read all runtime events from a `<session>.runtime.jsonl` log file.
///
/// If the final line is incomplete or corrupted (e.g. from an abrupt process termination),
/// the incomplete tail is gracefully ignored, returning all earlier valid rows.
pub fn read_runtime_log<T: serde::de::DeserializeOwned>(
    path: &Path,
) -> Result<Vec<T>, RuntimeLogError> {
    read_runtime_log_capped(path, MAX_RUNTIME_RECORD_BYTES)
}

/// Upper bound on one runtime record line. Writers emit far smaller records;
/// a longer line is corruption, such as a newline-free tail left by a crash,
/// and is never buffered whole (WOR-83).
const MAX_RUNTIME_RECORD_BYTES: usize = 64 * 1024 * 1024;

fn read_runtime_log_capped<T: serde::de::DeserializeOwned>(
    path: &Path,
    max_record_bytes: usize,
) -> Result<Vec<T>, RuntimeLogError> {
    if !path.is_file() {
        return Ok(Vec::new());
    }

    let mut reader = BufReader::new(File::open(path)?);
    let mut records = Vec::new();
    let mut line = Vec::new();
    let mut idx = 0usize;
    // Stream one bounded line at a time. Only the final line may be torn, so
    // after each line we check whether any input remains.
    while let Some(oversized) = next_line(&mut reader, &mut line, max_record_bytes)? {
        idx += 1;
        let is_last = reader.fill_buf()?.is_empty();
        // A crash can also cut a multi-byte character in the final line.
        let Some(text) = (!oversized)
            .then(|| std::str::from_utf8(&line).ok())
            .flatten()
        else {
            if is_last {
                break;
            }
            return Err(RuntimeLogError::CorruptRecord {
                line: idx,
                reason: if oversized {
                    format!("record is longer than {max_record_bytes} bytes")
                } else {
                    "record is not valid UTF-8".into()
                },
            });
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Parse into a Value first to inspect schema_version before typed conversion
        let value: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                // If this is the last line, tolerate an incomplete partial tail
                if is_last {
                    break;
                }
                return Err(RuntimeLogError::CorruptRecord {
                    line: idx,
                    reason: e.to_string(),
                });
            }
        };

        // Schema version check
        if let Some(ver) = value.get("schema_version").and_then(|v| v.as_u64()) {
            if ver > u64::from(CURRENT_RUNTIME_SCHEMA_VERSION) {
                return Err(RuntimeLogError::UnsupportedSchemaVersion {
                    found: u16::try_from(ver).unwrap_or(u16::MAX),
                    supported: CURRENT_RUNTIME_SCHEMA_VERSION,
                });
            }
        }

        match serde_json::from_value::<T>(value) {
            Ok(record) => records.push(record),
            Err(e) => {
                if is_last {
                    break;
                }
                return Err(RuntimeLogError::CorruptRecord {
                    line: idx,
                    reason: e.to_string(),
                });
            }
        }
    }

    Ok(records)
}

/// Reads the next line into `line`, keeping at most `max` bytes. Returns
/// `None` at end of input, otherwise whether the line was longer than `max`;
/// the rest of an oversized line is consumed and dropped, never buffered.
fn next_line(
    reader: &mut impl BufRead,
    line: &mut Vec<u8>,
    max: usize,
) -> std::io::Result<Option<bool>> {
    line.clear();
    let read = std::io::Read::take(&mut *reader, max as u64 + 1).read_until(b'\n', line)?;
    if read == 0 {
        return Ok(None);
    }
    if line.len() <= max || line.ends_with(b"\n") {
        return Ok(Some(false));
    }
    line.clear();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            break;
        }
        match available.iter().position(|byte| *byte == b'\n') {
            Some(end) => {
                reader.consume(end + 1);
                break;
            }
            None => {
                let len = available.len();
                reader.consume(len);
            }
        }
    }
    Ok(Some(true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use std::fs::{self, OpenOptions};
    use tempfile::tempdir;

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    struct DummyEnvelope {
        schema_version: u16,
        sequence: u64,
        session_id: String,
        event: String,
    }

    fn envelope_line(sequence: u64) -> String {
        format!(
            "{{\"schema_version\":1,\"sequence\":{sequence},\"session_id\":\"s\",\"event\":\"e\"}}\n"
        )
    }

    /// WOR-83: a newline-free tail longer than the record cap is a torn tail;
    /// earlier rows still replay, and the tail is never buffered past the cap.
    #[test]
    fn wor83_oversized_final_line_is_a_torn_tail() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("log.runtime.jsonl");
        let mut body = (0..3).map(envelope_line).collect::<String>();
        body.push_str(&"x".repeat(10_000));
        fs::write(&path, body).unwrap();
        let rows: Vec<DummyEnvelope> = read_runtime_log_capped(&path, 1024).unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn wor83_oversized_middle_line_is_corrupt_and_skipped_unbuffered() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("log.runtime.jsonl");
        let row = envelope_line(1);
        fs::write(&path, format!("{row}{}\n{row}", "x".repeat(10_000))).unwrap();
        let error = read_runtime_log_capped::<DummyEnvelope>(&path, 1024).unwrap_err();
        assert!(
            matches!(&error, RuntimeLogError::CorruptRecord { line: 2, reason }
                if reason.contains("longer than 1024")),
            "{error}"
        );
        let mut reader = BufReader::new(File::open(&path).unwrap());
        let mut line = Vec::new();
        assert_eq!(
            next_line(&mut reader, &mut line, 1024).unwrap(),
            Some(false)
        );
        assert_eq!(next_line(&mut reader, &mut line, 1024).unwrap(), Some(true));
        assert!(line.is_empty() && line.capacity() <= 2048);
        assert_eq!(
            next_line(&mut reader, &mut line, 1024).unwrap(),
            Some(false)
        );
        assert_eq!(line, row.as_bytes());
        assert_eq!(next_line(&mut reader, &mut line, 1024).unwrap(), None);
    }

    /// A crash can cut a multi-byte character in the final line.
    #[test]
    fn torn_multibyte_utf8_tail_is_tolerated() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("log.runtime.jsonl");
        let mut bytes = envelope_line(1).into_bytes();
        bytes.extend_from_slice(b"{\"event\":\"");
        bytes.extend_from_slice(&"\u{00e9}".as_bytes()[..1]);
        fs::write(&path, bytes).unwrap();
        let rows: Vec<DummyEnvelope> = read_runtime_log(&path).unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn test_runtime_log_path_derivation() {
        let session = Path::new("/var/davinci/sessions/2026-09-05_abc.jsonl");
        assert_eq!(
            runtime_log_path(session),
            PathBuf::from("/var/davinci/sessions/2026-09-05_abc.runtime.jsonl")
        );
    }

    #[test]
    fn test_append_and_read_roundtrip() {
        let dir = tempdir().unwrap();
        let log_file = dir.path().join("session-1.runtime.jsonl");

        let mut writer = RuntimeLogWriter::open(&log_file).unwrap();
        let r1 = DummyEnvelope {
            schema_version: 1,
            sequence: 1,
            session_id: "s1".into(),
            event: "session_start".into(),
        };
        let r2 = DummyEnvelope {
            schema_version: 1,
            sequence: 2,
            session_id: "s1".into(),
            event: "task_create".into(),
        };

        writer.append(&r1).unwrap();
        writer.append(&r2).unwrap();

        let read: Vec<DummyEnvelope> = read_runtime_log(&log_file).unwrap();
        assert_eq!(read.len(), 2);
        assert_eq!(read[0], r1);
        assert_eq!(read[1], r2);
    }

    #[test]
    fn writer_repairs_a_torn_tail_before_appending() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("s.runtime.jsonl");
        fs::write(
            &path,
            "{\"schema_version\":1,\"a\":1}\n{\"schema_version\":1,\"a\":",
        )
        .unwrap();
        let mut writer = RuntimeLogWriter::open(&path).unwrap();
        writer
            .append(&serde_json::json!({"schema_version": 1, "a": 2}))
            .unwrap();
        let rows: Vec<serde_json::Value> = read_runtime_log(&path).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn huge_schema_version_is_unsupported_not_truncated() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("s.runtime.jsonl");
        fs::write(&path, "{\"schema_version\":65537}\n").unwrap();
        let err = read_runtime_log::<serde_json::Value>(&path).unwrap_err();
        assert!(matches!(
            err,
            RuntimeLogError::UnsupportedSchemaVersion { .. }
        ));
    }

    #[test]
    fn test_corrupt_final_tail_recovery() {
        let dir = tempdir().unwrap();
        let log_file = dir.path().join("session-crash.runtime.jsonl");

        let mut writer = RuntimeLogWriter::open(&log_file).unwrap();
        let r1 = DummyEnvelope {
            schema_version: 1,
            sequence: 1,
            session_id: "s1".into(),
            event: "init".into(),
        };
        writer.append(&r1).unwrap();
        drop(writer);

        // Append a corrupted partial JSON line
        {
            let mut file = OpenOptions::new().append(true).open(&log_file).unwrap();
            writeln!(file, "{{\"schema_version\": 1, \"sequence\": 2, \"session_id\": \"s1\", \"event\": \"crashed-").unwrap();
        }

        let read: Vec<DummyEnvelope> = read_runtime_log(&log_file).unwrap();
        assert_eq!(
            read.len(),
            1,
            "Must recover earlier valid records despite corrupted tail"
        );
        assert_eq!(read[0], r1);
    }

    #[test]
    fn test_corrupt_middle_line_fails() {
        let dir = tempdir().unwrap();
        let log_file = dir.path().join("session-bad-middle.runtime.jsonl");

        let mut file = File::create(&log_file).unwrap();
        writeln!(file, "{{\"schema_version\": 1, \"sequence\": 1, \"session_id\": \"s1\", \"event\": \"init\"}}").unwrap();
        writeln!(file, "{{corrupted line in the middle}}").unwrap();
        writeln!(file, "{{\"schema_version\": 1, \"sequence\": 2, \"session_id\": \"s1\", \"event\": \"done\"}}").unwrap();

        let res: Result<Vec<DummyEnvelope>, _> = read_runtime_log(&log_file);
        assert!(matches!(
            res,
            Err(RuntimeLogError::CorruptRecord { line: 2, .. })
        ));
    }

    #[test]
    fn test_unsupported_schema_version_rejected() {
        let dir = tempdir().unwrap();
        let log_file = dir.path().join("session-future.runtime.jsonl");

        let mut writer = RuntimeLogWriter::open(&log_file).unwrap();
        let r = DummyEnvelope {
            schema_version: 99,
            sequence: 1,
            session_id: "s1".into(),
            event: "future".into(),
        };
        writer.append(&r).unwrap();

        let res: Result<Vec<DummyEnvelope>, _> = read_runtime_log(&log_file);
        assert!(matches!(
            res,
            Err(RuntimeLogError::UnsupportedSchemaVersion {
                found: 99,
                supported: 1
            })
        ));
    }
}
