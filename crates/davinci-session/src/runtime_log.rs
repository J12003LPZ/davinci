//! Versioned JSONL sidecar log for runtime lifecycle events (`<session>.runtime.jsonl`).
//!
//! Stores `RuntimeEventEnvelope` rows alongside session JSONL files without modifying
//! the upstream session format.

use std::fs::{self, File, OpenOptions};
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
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self { path, file })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append<T: serde::Serialize>(&mut self, record: &T) -> Result<(), RuntimeLogError> {
        let line = serde_json::to_string(record)?;
        writeln!(self.file, "{line}")?;
        self.file.flush()?;
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
    if !path.is_file() {
        return Ok(Vec::new());
    }

    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let raw_lines: Vec<String> = reader.lines().collect::<Result<_, _>>()?;

    let mut records = Vec::new();
    let total = raw_lines.len();

    for (idx, line) in raw_lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Parse into a Value first to inspect schema_version before typed conversion
        let value: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                // If this is the last line, tolerate an incomplete partial tail
                if idx == total - 1 {
                    break;
                }
                return Err(RuntimeLogError::CorruptRecord {
                    line: idx + 1,
                    reason: e.to_string(),
                });
            }
        };

        // Schema version check
        if let Some(ver) = value.get("schema_version").and_then(|v| v.as_u64()) {
            if ver as u16 > CURRENT_RUNTIME_SCHEMA_VERSION {
                return Err(RuntimeLogError::UnsupportedSchemaVersion {
                    found: ver as u16,
                    supported: CURRENT_RUNTIME_SCHEMA_VERSION,
                });
            }
        }

        match serde_json::from_value::<T>(value) {
            Ok(record) => records.push(record),
            Err(e) => {
                if idx == total - 1 {
                    break;
                }
                return Err(RuntimeLogError::CorruptRecord {
                    line: idx + 1,
                    reason: e.to_string(),
                });
            }
        }
    }

    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use tempfile::tempdir;

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    struct DummyEnvelope {
        schema_version: u16,
        sequence: u64,
        session_id: String,
        event: String,
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
