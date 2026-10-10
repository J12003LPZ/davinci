//! JSONL SessionRepo matching `vendor/pi/packages/agent/src/harness/session/jsonl`.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde_json::Value;
use uuid::Uuid;

use crate::codec::{encode_header, encode_mutation, parse_header, parse_mutation};
use crate::repo::{
    custom_entry, user_message_entry, BranchBounds, EntryQuery, ForkOptions, LogItem, LogOptions,
    PlannedEntryAppend, RecordQuery, Session, SessionStats,
};
use crate::types::{JsonlV4Header, LaneRecord, SessionEntry};
use crate::{now_ms, LanePointer, SessionError, SessionMutation};

/// Held while a session id is checked and its file published.
const NAMES_LOCK: &str = ".sessions.lock";
/// How long a loader or creator waits for another handle's short critical
/// section (one append, one publish) before giving up.
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

const SESSION_ID_PATTERN_MSG: &str = "Session id must be non-empty, contain only alphanumeric characters, '-', '_', and '.', and start and end with an alphanumeric character";

pub fn validate_session_id(id: &str) -> Result<(), SessionError> {
    let valid = id
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_alphanumeric())
        && id
            .chars()
            .last()
            .is_some_and(|ch| ch.is_ascii_alphanumeric())
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'));
    if !valid {
        return Err(SessionError::invalid_payload(SESSION_ID_PATTERN_MSG));
    }
    Ok(())
}

/// TS `jsonlSessionDirectoryName` (not the coding-agent `--` path encoder).
pub fn jsonl_session_directory_name(cwd: &str) -> String {
    let stripped = cwd.trim_start_matches(['/', '\\']);
    format!("--{}--", stripped.replace(['/', '\\', ':'], "-"))
}

pub fn session_file_name(created_at_ms: u64, id: &str) -> String {
    format!("{}_{id}.jsonl", utc_iso_dashed(created_at_ms))
}

pub fn expected_session_path(root: &Path, cwd: &str, created_at_ms: u64, id: &str) -> PathBuf {
    root.join(jsonl_session_directory_name(cwd))
        .join(session_file_name(created_at_ms, id))
}

fn utc_iso_dashed(ms: u64) -> String {
    let (year, month, day, hour, minute, second, milli) = unix_ms_to_utc(ms);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}-{minute:02}-{second:02}-{milli:03}Z")
}

/// UTC calendar date using the same conversion as session filenames.
pub fn utc_date_from_unix_ms(ms: u64) -> String {
    let (year, month, day, ..) = unix_ms_to_utc(ms);
    format!("{year:04}-{month:02}-{day:02}")
}

fn unix_ms_to_utc(ms: u64) -> (i32, u32, u32, u32, u32, u32, u32) {
    let day_ms = 86_400_000u64;
    let days = (ms / day_ms) as i64;
    let rem = ms % day_ms;
    let hour = (rem / 3_600_000) as u32;
    let rem = rem % 3_600_000;
    let minute = (rem / 60_000) as u32;
    let rem = rem % 60_000;
    let second = (rem / 1000) as u32;
    let milli = (rem % 1000) as u32;
    let (year, month, day) = civil_from_days(days);
    (year, month, day, hour, minute, second, milli)
}

/// Howard Hinnant civil-from-days: Unix epoch day 0 is 1970-01-01.
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

fn modified_at_ms(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn invalid_file(path: &Path, line: usize, message: impl std::fmt::Display) -> SessionError {
    SessionError::invalid_entry(format!(
        "Invalid JSONL v4 session {}: line {line} {message}",
        path.display()
    ))
}

#[derive(Debug, Clone)]
pub struct JsonlCreateOptions {
    pub id: Option<String>,
    pub cwd: String,
    pub parent_session_id: Option<String>,
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JsonlSessionInfo {
    pub id: String,
    pub created_at: u64,
    pub parent_session_id: Option<String>,
    pub cwd: String,
    pub path: PathBuf,
    pub modified_at: u64,
    pub source_format: u8,
    pub metadata: Option<Value>,
}

impl JsonlSessionInfo {
    fn from_header(header: &JsonlV4Header, path: &Path) -> Self {
        Self {
            id: header.id.clone(),
            created_at: header.created_at,
            parent_session_id: header.parent_session_id.clone(),
            cwd: header.cwd.clone(),
            path: path.to_path_buf(),
            modified_at: modified_at_ms(path),
            source_format: if header.legacy_parent_session_path.is_some() {
                3
            } else {
                4
            },
            metadata: header.metadata.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct JsonlSessionRepo {
    sessions_root: PathBuf,
}

impl JsonlSessionRepo {
    pub fn new(sessions_root: impl Into<PathBuf>) -> Self {
        Self {
            sessions_root: sessions_root.into(),
        }
    }

    pub fn create(&self, options: JsonlCreateOptions) -> Result<JsonlStoredSession, SessionError> {
        let id = options
            .id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        validate_session_id(&id)?;
        let cwd = options.cwd;
        let dir = self.sessions_root.join(jsonl_session_directory_name(&cwd));
        let _naming = self.lock_names(&dir)?;
        if self.session_id_exists(&id, &cwd)? {
            return Err(SessionError::already_exists(format!(
                "Session already exists: {id}"
            )));
        }
        let created_at = now_ms();
        let path = dir.join(session_file_name(created_at, &id));
        let header = JsonlV4Header {
            kind: "header".into(),
            version: 4,
            id: id.clone(),
            created_at,
            cwd: cwd.clone(),
            parent_session_id: options.parent_session_id.clone(),
            legacy_parent_session_path: None,
            metadata: options.metadata.clone(),
        };
        let write_header = || -> std::io::Result<()> {
            let mut file = davinci_sys::fs::create_new_private(&path)?;
            file.write_all(encode_header(&header).as_bytes())?;
            file.sync_all()?;
            crate::sync_parent(&path)
        };
        write_header().map_err(|err| {
            SessionError::storage(format!(
                "Failed to initialize session {}: {err}",
                path.display()
            ))
        })?;
        Ok(JsonlStoredSession {
            persistence_error: None,
            durable_len: encode_header(&header).len() as u64,
            session: Session::with_metadata(id, created_at, options.parent_session_id),
            info: JsonlSessionInfo::from_header(&header, &path),
        })
    }

    pub fn open(&self, info: &JsonlSessionInfo) -> Result<JsonlStoredSession, SessionError> {
        if !info.path.exists() {
            return Err(SessionError::not_found(format!(
                "Session not found: {}",
                info.id
            )));
        }
        let loaded = JsonlStoredSession::load(&info.path)?;
        if loaded.info.id != info.id {
            return Err(SessionError::invalid_entry(format!(
                "Session id does not match header: {}",
                info.id
            )));
        }
        Ok(loaded)
    }

    pub fn open_path(&self, path: &Path) -> Result<JsonlStoredSession, SessionError> {
        JsonlStoredSession::load(path)
    }

    pub fn list(&self, cwd: Option<&str>) -> Result<Vec<JsonlSessionInfo>, SessionError> {
        let mut listed = Vec::new();
        let directories = if let Some(cwd) = cwd {
            vec![self.sessions_root.join(jsonl_session_directory_name(cwd))]
        } else {
            crate::discovery::session_subdirectories(&self.sessions_root).unwrap_or_default()
        };
        for directory in directories {
            if !directory.exists() {
                continue;
            }
            let Ok(entries) = fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl")
                    || !entry.file_type().is_ok_and(|kind| kind.is_file())
                {
                    continue;
                }
                let Some(first) = crate::bounded::read_header_line(&path) else {
                    continue;
                };
                let Ok(header) = parse_header(&first) else {
                    continue;
                };
                listed.push(JsonlSessionInfo::from_header(&header, &path));
            }
        }
        listed.sort_by(|left, right| right.modified_at.cmp(&left.modified_at));
        Ok(listed)
    }

    pub fn delete(&self, info: &JsonlSessionInfo) -> Result<(), SessionError> {
        if info.path.exists() {
            fs::remove_file(&info.path).map_err(|err| {
                SessionError::storage(format!(
                    "Failed to delete session {}: {err}",
                    info.path.display()
                ))
            })?;
        }
        Ok(())
    }

    pub fn fork(
        &self,
        source: &JsonlStoredSession,
        options: &ForkOptions,
        cwd: &str,
    ) -> Result<JsonlStoredSession, SessionError> {
        let id = options
            .id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        validate_session_id(&id)?;
        let mutations = source.session.state().create_fork_mutations(options)?;
        let dir = self.sessions_root.join(jsonl_session_directory_name(cwd));
        let _naming = self.lock_names(&dir)?;
        if self.session_id_exists(&id, cwd)? {
            return Err(SessionError::already_exists(format!(
                "Session already exists: {id}"
            )));
        }
        let created_at = now_ms();
        let path = dir.join(session_file_name(created_at, &id));
        let header = JsonlV4Header {
            kind: "header".into(),
            version: 4,
            id: id.clone(),
            created_at,
            cwd: cwd.to_string(),
            parent_session_id: options
                .parent_session_id
                .clone()
                .or_else(|| Some(source.info.id.clone())),
            legacy_parent_session_path: None,
            metadata: None,
        };
        publish_atomically(&path, |temp_path| {
            let mut body = encode_header(&header);
            for item in &mutations {
                body.push_str(&encode_mutation(&item.clone().into_mutation()));
            }
            fs::write(temp_path, body).map_err(|err| {
                SessionError::storage(format!(
                    "Failed to stage fork {}: {err}",
                    temp_path.display()
                ))
            })
        })?;
        JsonlStoredSession::load(&path)
    }

    /// Serialize id allocation in one sessions directory. "Is this id free?"
    /// and "publish the file" are two steps, and the filename carries a
    /// timestamp, so without this two creators of one id both pass the check
    /// and publish twice (or, same millisecond, one rename replaces the other).
    fn lock_names(&self, dir: &Path) -> Result<davinci_sys::lock::ExclusiveFileLock, SessionError> {
        davinci_sys::fs::create_private_dir_all(dir).map_err(|err| {
            SessionError::storage(format!("Failed to create sessions directory: {err}"))
        })?;
        davinci_sys::lock::ExclusiveFileLock::acquire(&dir.join(NAMES_LOCK), LOCK_WAIT).map_err(
            |err| SessionError::storage(format!("Unable to lock sessions directory: {err}")),
        )
    }

    fn session_id_exists(&self, id: &str, cwd: &str) -> Result<bool, SessionError> {
        let directory = self.sessions_root.join(jsonl_session_directory_name(cwd));
        if !directory.exists() {
            return Ok(false);
        }
        let suffix = format!("_{id}.jsonl");
        let entries = fs::read_dir(&directory).map_err(|err| {
            SessionError::storage(format!(
                "Failed to list sessions directory {}: {err}",
                directory.display()
            ))
        })?;
        Ok(entries.flatten().any(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.ends_with(&suffix))
        }))
    }
}

pub(crate) fn publish_atomically(
    destination: &Path,
    populate: impl FnOnce(&Path) -> Result<(), SessionError>,
) -> Result<(), SessionError> {
    let temp_path = destination.with_file_name(format!(".session-{}.tmp", Uuid::new_v4()));
    let result = (|| {
        populate(&temp_path)?;
        // Transcripts are private: the published file is owner-only whatever
        // the umask, and a rewrite also tightens a legacy world-readable one.
        davinci_sys::fs::restrict_to_owner(&temp_path).map_err(|err| {
            SessionError::storage(format!("Failed to restrict staged session: {err}"))
        })?;
        OpenOptions::new()
            .write(true)
            .open(&temp_path)
            .and_then(|file| file.sync_all())
            .map_err(|err| {
                SessionError::storage(format!("Failed to flush staged session: {err}"))
            })?;
        fs::rename(&temp_path, destination).map_err(|err| {
            SessionError::storage(format!(
                "Failed to publish staged file {}: {err}",
                destination.display()
            ))
        })?;
        crate::sync_parent(destination).map_err(|err| {
            SessionError::storage(format!("Failed to flush session directory: {err}"))
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

#[derive(Debug, Clone)]
pub struct JsonlStoredSession {
    pub session: Session,
    pub info: JsonlSessionInfo,
    persistence_error: Option<String>,
    /// File length this handle last observed as its own durable tail. Appends
    /// are fenced on it, so a stale handle (another writer appended since it
    /// loaded) is rejected instead of reusing a sequence number.
    durable_len: u64,
}

/// What `load_with` may do about a tail that needs repair.
enum Tail {
    Detect,
    /// Holds the session lock for the rewrite.
    Repair(#[allow(dead_code)] davinci_sys::lock::ExclusiveFileLock),
    ReadOnly,
}

/// How long a loader waits for an in-flight append before treating the
/// lock as held by a long-lived writer. Appends hold it for milliseconds.
const REPAIR_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(1);

fn session_lock_path(path: &Path) -> PathBuf {
    let mut lock_path = path.as_os_str().to_owned();
    lock_path.push(".lock");
    PathBuf::from(lock_path)
}

impl JsonlStoredSession {
    pub fn load(path: &Path) -> Result<Self, SessionError> {
        if let Some(loaded) = Self::load_with(path, &Tail::Detect)? {
            return Ok(loaded);
        }
        // The tail needs repair. Appenders write under the session lock, so a
        // torn tail seen without it may be a line another handle is writing
        // right now, and a rewrite from this snapshot would erase it (and
        // anything appended after the read). Re-read under the lock, where
        // the tail is stable, and repair only what is still torn then.
        // A writer that keeps the lock (a live session in another handle)
        // owns its tail: load the complete records read-only instead of
        // waiting on it or rewriting under it.
        let tail = match davinci_sys::lock::ExclusiveFileLock::acquire(
            &session_lock_path(path),
            REPAIR_LOCK_WAIT,
        ) {
            Ok(lock) => Tail::Repair(lock),
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => Tail::ReadOnly,
            Err(err) => {
                return Err(SessionError::storage(format!(
                    "Unable to lock session {} for tail repair: {err}",
                    path.display()
                )))
            }
        };
        Self::load_with(path, &tail)?
            .ok_or_else(|| SessionError::storage("session tail repair did not complete"))
    }

    /// Parse `path`. What happens to a tail that needs rewriting depends on
    /// `tail`: report it (`None`), rewrite it (the lock is held), or leave
    /// it and return the complete records with writes refused.
    fn load_with(path: &Path, tail: &Tail) -> Result<Option<Self>, SessionError> {
        let (content, torn_utf8_tail) = crate::read_session_text(path).map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                SessionError::not_found(format!("Session not found: {}", path.display()))
            } else {
                SessionError::storage(format!("Failed to read session {}: {err}", path.display()))
            }
        })?;
        let mut physical_lines: Vec<&str> = content.split('\n').collect();
        if physical_lines.last() == Some(&"") {
            physical_lines.pop();
        }
        if physical_lines.is_empty() || physical_lines[0].is_empty() {
            return Err(invalid_file(path, 1, "is missing a header"));
        }
        let header = parse_header(physical_lines[0]).map_err(|err| invalid_file(path, 1, err))?;
        let mut session = Session::with_metadata(
            header.id.clone(),
            header.created_at,
            header.parent_session_id.clone(),
        );
        for (index, line) in physical_lines.iter().enumerate().skip(1) {
            match parse_mutation(line) {
                Ok(mutation) => {
                    if let Err(err) = session.apply_mutation(mutation) {
                        if err.code == "invalid_entry" {
                            return Err(invalid_file(path, index + 1, err));
                        }
                        return Err(err);
                    }
                }
                Err(err) if err.kind == "syntax" && index + 1 == physical_lines.len() => {
                    let valid_prefix = format!("{}\n", physical_lines[..index].join("\n"));
                    match tail {
                        Tail::Detect => return Ok(None),
                        Tail::ReadOnly => {
                            return Ok(Some(Self::read_only(session, &header, path)));
                        }
                        Tail::Repair(_) => {}
                    }
                    publish_atomically(path, |temp_path| {
                        fs::write(temp_path, &valid_prefix).map_err(|write_err| {
                            SessionError::storage(format!(
                                "Failed to stage torn-tail repair {}: {write_err}",
                                path.display()
                            ))
                        })
                    })?;
                    return Ok(Some(Self {
                        persistence_error: None,
                        durable_len: valid_prefix.len() as u64,
                        session,
                        info: JsonlSessionInfo::from_header(&header, path),
                    }));
                }
                Err(err) => return Err(invalid_file(path, index + 1, err)),
            }
        }
        if torn_utf8_tail || !content.ends_with('\n') {
            match tail {
                Tail::Detect => return Ok(None),
                Tail::ReadOnly => return Ok(Some(Self::read_only(session, &header, path))),
                Tail::Repair(_) => {}
            }
        }
        if torn_utf8_tail {
            // `content` is the complete-record prefix; drop the torn bytes.
            publish_atomically(path, |temp_path| {
                fs::write(temp_path, &content).map_err(|write_err| {
                    SessionError::storage(format!(
                        "Failed to stage torn-tail repair {}: {write_err}",
                        path.display()
                    ))
                })
            })?;
        } else if !content.ends_with('\n') {
            let mut file = OpenOptions::new().append(true).open(path).map_err(|err| {
                SessionError::storage(format!(
                    "Failed to repair unterminated session tail {}: {err}",
                    path.display()
                ))
            })?;
            file.write_all(b"\n")
                .and_then(|()| file.sync_all())
                .map_err(|err| {
                    SessionError::storage(format!(
                        "Failed to repair unterminated session tail {}: {err}",
                        path.display()
                    ))
                })?;
        }
        Ok(Some(Self {
            persistence_error: None,
            durable_len: content.len() as u64 + u64::from(!content.ends_with('\n')),
            session,
            info: JsonlSessionInfo::from_header(&header, path),
        }))
    }

    fn read_only(session: Session, header: &JsonlV4Header, path: &Path) -> Self {
        Self {
            persistence_error: Some(
                "another handle is writing this session's tail; reopen it to write".into(),
            ),
            durable_len: 0,
            session,
            info: JsonlSessionInfo::from_header(header, path),
        }
    }

    pub fn info(&self) -> &JsonlSessionInfo {
        &self.info
    }

    pub fn get_lanes(&self) -> Vec<LanePointer> {
        self.session.get_lanes()
    }

    pub fn get_name(&self) -> Option<&str> {
        self.session.get_name()
    }

    pub fn get_label(&self, id: &str) -> Option<&str> {
        self.session.get_label(id)
    }

    pub fn get_stats(&self) -> &SessionStats {
        self.session.get_stats()
    }

    pub fn get_leaf_id(&self) -> Option<String> {
        self.session.get_leaf_id()
    }

    pub fn get_entry(&self, id: &str) -> Option<&SessionEntry> {
        self.session.get_entry(id)
    }

    pub fn get_log(&self, options: &LogOptions) -> Result<Vec<LogItem>, SessionError> {
        self.session.get_log(options)
    }

    pub fn find_entries(&self, query: &EntryQuery) -> Result<Vec<SessionEntry>, SessionError> {
        self.session.find_entries(query)
    }

    pub fn find_entries_on_branch(
        &self,
        query: &EntryQuery,
        bounds: &BranchBounds,
    ) -> Result<Vec<SessionEntry>, SessionError> {
        self.session.find_entries_on_branch(query, bounds)
    }

    pub fn find_records(&self, query: &RecordQuery) -> Result<Vec<LaneRecord>, SessionError> {
        self.session.find_records(query)
    }

    pub fn find_open_operations(
        &self,
        lane: &str,
        limit: Option<usize>,
    ) -> Result<Vec<LaneRecord>, SessionError> {
        self.session.find_open_operations(lane, limit)
    }

    pub fn append_entry(
        &mut self,
        entry: SessionEntry,
        lane: &str,
    ) -> Result<SessionEntry, SessionError> {
        self.require_writable()?;
        let (mutation, entry) = self.session.plan_entry(entry, lane)?;
        self.commit_mutation(mutation)?;
        Ok(entry)
    }

    pub fn append_entry_once(
        &mut self,
        event_id: &str,
        expected_parent_id: Option<&str>,
        entry: SessionEntry,
        lane: &str,
    ) -> Result<SessionEntry, SessionError> {
        self.require_writable()?;
        match self
            .session
            .plan_entry_once(lane, event_id, expected_parent_id, entry)?
        {
            PlannedEntryAppend::Existing(entry) => Ok(entry),
            PlannedEntryAppend::Mutation { mutation, entry } => {
                self.commit_mutation(mutation)?;
                Ok(entry)
            }
        }
    }

    pub fn append_custom_entry(
        &mut self,
        custom_type: &str,
        data: Value,
    ) -> Result<String, SessionError> {
        let entry = self.append_entry(
            custom_entry(&Uuid::new_v4().to_string(), custom_type, data),
            "main",
        )?;
        Ok(entry.id)
    }

    pub fn append_message(&mut self, text: &str) -> Result<String, SessionError> {
        let entry = self.append_entry(
            user_message_entry(&Uuid::new_v4().to_string(), text),
            "main",
        )?;
        Ok(entry.id)
    }

    pub fn append_record(&mut self, record: LaneRecord) -> Result<LaneRecord, SessionError> {
        self.require_writable()?;
        let (mutation, record) = self.session.plan_record(record)?;
        self.commit_mutation(mutation)?;
        Ok(record)
    }

    pub fn create_lane(&mut self, lane: &str, at: Option<&str>) -> Result<(), SessionError> {
        self.require_writable()?;
        let mutation = self.session.plan_create_lane(lane, at)?;
        self.commit_mutation(mutation)
    }

    pub fn move_lane(&mut self, lane: &str, to: Option<&str>) -> Result<(), SessionError> {
        self.require_writable()?;
        let mutation = self.session.plan_move_lane(lane, to)?;
        self.commit_mutation(mutation)
    }

    pub fn set_name(&mut self, name: Option<&str>) -> Result<(), SessionError> {
        self.require_writable()?;
        let mutation = self.session.plan_name(name);
        self.commit_mutation(mutation)
    }

    pub fn set_label(&mut self, id: &str, label: Option<&str>) -> Result<(), SessionError> {
        self.require_writable()?;
        let mutation = self.session.plan_label(id, label)?;
        self.commit_mutation(mutation)
    }

    fn require_writable(&self) -> Result<(), SessionError> {
        if let Some(error) = &self.persistence_error {
            return Err(SessionError::storage(format!(
                "Session recovery required: {error}"
            )));
        }
        Ok(())
    }

    fn commit_mutation(&mut self, mutation: SessionMutation) -> Result<(), SessionError> {
        self.append_mutation(mutation.clone())
            .inspect_err(|error| {
                self.persistence_error = Some(error.to_string());
            })?;
        if let Err(error) = self.session.apply_mutation(mutation) {
            self.persistence_error = Some(error.to_string());
            return Err(SessionError::storage(format!(
                "Session mutation was persisted but could not be applied in memory; reopen the session: {error}"
            )));
        }
        Ok(())
    }

    fn append_mutation(&mut self, mutation: SessionMutation) -> Result<(), SessionError> {
        let _lock =
            davinci_sys::lock::ExclusiveFileLock::try_acquire(&session_lock_path(&self.info.path))
                .map_err(|err| {
                    if err.kind() == std::io::ErrorKind::WouldBlock {
                        SessionError::storage(format!(
                            "Session {} is being written by another handle; reopen it",
                            self.info.path.display()
                        ))
                    } else {
                        SessionError::storage(format!("Unable to lock session: {err}"))
                    }
                })?;
        let current_len = fs::metadata(&self.info.path)
            .map_err(|err| {
                SessionError::storage(format!(
                    "Failed to append session {}: {err}",
                    self.info.path.display()
                ))
            })?
            .len();
        if current_len != self.durable_len {
            return Err(SessionError::storage(format!(
                "Session {} changed since this handle loaded it; reopen it before writing",
                self.info.path.display()
            )));
        }
        let line = encode_mutation(&mutation);
        let mut file = OpenOptions::new()
            .append(true)
            .open(&self.info.path)
            .map_err(|err| {
                SessionError::storage(format!(
                    "Failed to append session {}: {err}",
                    self.info.path.display()
                ))
            })?;
        file.write_all(line.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|err| {
                SessionError::storage(format!(
                    "Failed to append session {}: {err}",
                    self.info.path.display()
                ))
            })?;
        self.durable_len += line.len() as u64;
        self.info.modified_at = now_ms();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::{operation_started, EntryOrder, ForkScope};
    use tempfile::tempdir;

    #[test]
    fn failed_stored_session_writes_preserve_state() {
        for operation in ["append", "record", "lane", "move", "name", "label"] {
            let dir = tempdir().unwrap();
            let repo = JsonlSessionRepo::new(dir.path());
            let mut session = repo
                .create(JsonlCreateOptions {
                    id: Some("fixture".into()),
                    cwd: "/fixture".into(),
                    parent_session_id: None,
                    metadata: None,
                })
                .unwrap();
            let entry = session.append_message("first").unwrap();
            let original = session.get_log(&LogOptions::default()).unwrap();
            let path = session.info.path.clone();
            let backup = path.with_extension("backup");
            fs::rename(&path, &backup).unwrap();
            fs::create_dir(&path).unwrap();
            let result = match operation {
                "append" => session.append_message("lost").map(|_| ()),
                "record" => session
                    .append_record(operation_started("run", "main", "lost"))
                    .map(|_| ()),
                "lane" => session.create_lane("lost", Some(&entry)),
                "move" => session.move_lane("main", None),
                "name" => session.set_name(Some("lost")),
                _ => session.set_label(&entry, Some("lost")),
            };
            assert!(result.is_err());
            assert_eq!(
                session.get_log(&LogOptions::default()).unwrap(),
                original,
                "{operation}"
            );
            fs::remove_dir(&path).unwrap();
            fs::rename(&backup, &path).unwrap();
            assert!(session.append_message("must reopen").is_err());
            let mut reopened = repo.open(&session.info).unwrap();
            reopened.append_message("recovered").unwrap();
            assert_eq!(reopened.get_stats().message_count, 2);
        }
    }

    #[cfg(unix)]
    #[test]
    fn repo_sessions_and_forks_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        let dir = tempdir().unwrap();
        let repo = JsonlSessionRepo::new(dir.path());
        let mut session = repo
            .create(JsonlCreateOptions {
                id: Some("private".into()),
                cwd: "/fixture".into(),
                parent_session_id: None,
                metadata: None,
            })
            .unwrap();
        session.append_message("secret").unwrap();
        assert_eq!(mode(session.info.path.parent().unwrap()), 0o700);
        assert_eq!(mode(&session.info.path), 0o600);
        let forked = repo
            .fork(&session, &ForkOptions::default(), "/fixture")
            .unwrap();
        assert_eq!(mode(&forked.info.path), 0o600);
    }

    #[test]
    fn audit_listing_reads_only_the_header() {
        let dir = tempdir().unwrap();
        let repo = JsonlSessionRepo::new(dir.path());
        let session = repo
            .create(JsonlCreateOptions {
                id: Some("header-only".into()),
                cwd: "/fixture".into(),
                parent_session_id: None,
                metadata: None,
            })
            .unwrap();
        // A damaged or unfinished transcript must not hide a valid header.
        let mut file = OpenOptions::new()
            .append(true)
            .open(&session.info.path)
            .unwrap();
        file.write_all(&[0xff; 64 * 1024]).unwrap();
        drop(file);
        let listed = repo.list(Some("/fixture")).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "header-only");
    }

    #[test]
    fn repeated_appends_do_not_require_session_clones() {
        let dir = tempdir().unwrap();
        let repo = JsonlSessionRepo::new(dir.path());
        let mut session = repo
            .create(JsonlCreateOptions {
                id: Some("many".into()),
                cwd: "/fixture".into(),
                parent_session_id: None,
                metadata: None,
            })
            .unwrap();
        for index in 0..2_000 {
            session.append_message(&format!("message-{index}")).unwrap();
        }
        assert_eq!(session.get_stats().message_count, 2_000);
        let log = session.get_log(&LogOptions::default()).unwrap();
        assert_eq!(log.len(), 2_000);
        assert_eq!(log.last().map(LogItem::seq), Some(2_000));
    }

    #[test]
    fn session_file_name_matches_ts_iso_dashes() {
        assert_eq!(
            session_file_name(1_767_225_600_000, "metadata"),
            "2026-01-01T00-00-00-000Z_metadata.jsonl"
        );
        assert_eq!(
            jsonl_session_directory_name("/tmp/workspace/project"),
            "--tmp-workspace-project--"
        );
    }

    #[test]
    fn utc_date_uses_calendar_boundaries_and_leap_days() {
        for (ms, expected) in [
            (0, "1970-01-01"),
            (86_399_999, "1970-01-01"),
            (86_400_000, "1970-01-02"),
            (951_782_400_000, "2000-02-29"),
            (1_767_225_599_999, "2025-12-31"),
            (1_767_225_600_000, "2026-01-01"),
        ] {
            assert_eq!(utc_date_from_unix_ms(ms), expected);
            assert!(session_file_name(ms, "date").starts_with(expected));
        }
    }

    #[test]
    fn rejects_session_ids_that_cannot_be_filenames() {
        let dir = tempdir().unwrap();
        let repo = JsonlSessionRepo::new(dir.path());
        assert_eq!(
            repo.create(JsonlCreateOptions {
                id: Some("../escape".into()),
                cwd: dir.path().to_string_lossy().into(),
                parent_session_id: None,
                metadata: None,
            })
            .unwrap_err()
            .code,
            "invalid_payload"
        );
    }

    #[test]
    fn allows_same_id_in_different_cwds_and_lists_by_cwd() {
        let dir = tempdir().unwrap();
        let repo = JsonlSessionRepo::new(dir.path());
        let first = dir.path().join("workspaces/first");
        let second = dir.path().join("workspaces/second");
        let a = repo
            .create(JsonlCreateOptions {
                id: Some("shared".into()),
                cwd: first.to_string_lossy().into(),
                parent_session_id: None,
                metadata: None,
            })
            .unwrap();
        let b = repo
            .create(JsonlCreateOptions {
                id: Some("shared".into()),
                cwd: second.to_string_lossy().into(),
                parent_session_id: None,
                metadata: None,
            })
            .unwrap();
        assert_eq!(a.info.cwd, first.to_string_lossy());
        assert_eq!(b.info.cwd, second.to_string_lossy());
        let listed = repo.list(None).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(repo.list(Some(&first.to_string_lossy())).unwrap().len(), 1);
        assert_eq!(
            repo.create(JsonlCreateOptions {
                id: Some("shared".into()),
                cwd: first.to_string_lossy().into(),
                parent_session_id: None,
                metadata: None,
            })
            .unwrap_err()
            .code,
            "already_exists"
        );
    }

    #[test]
    fn writes_one_line_per_mutation_and_restores_shared_sequence() {
        let dir = tempdir().unwrap();
        let repo = JsonlSessionRepo::new(dir.path());
        let cwd = dir.path().to_string_lossy().to_string();
        let mut session = repo
            .create(JsonlCreateOptions {
                id: Some("session".into()),
                cwd: cwd.clone(),
                parent_session_id: None,
                metadata: None,
            })
            .unwrap();
        let info = session.info.clone();
        let entry_id = session
            .append_custom_entry("note", serde_json::json!({ "value": 1 }))
            .unwrap();
        session.create_lane("thread", Some(&entry_id)).unwrap();
        session
            .append_record(operation_started("run", "thread", "run"))
            .unwrap();
        session.set_name(Some("Example")).unwrap();
        session.set_label(&entry_id, Some("checkpoint")).unwrap();
        session.move_lane("main", None).unwrap();

        let raw = std::fs::read_to_string(&info.path).unwrap();
        let kinds: Vec<_> = raw
            .trim_end()
            .lines()
            .map(|line| {
                serde_json::from_str::<Value>(line).unwrap()["kind"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(
            kinds,
            ["header", "entry", "lane", "record", "fact", "fact", "lane"]
        );
        let seqs: Vec<_> = raw
            .trim_end()
            .lines()
            .skip(1)
            .map(|line| {
                serde_json::from_str::<Value>(line).unwrap()["seq"]
                    .as_u64()
                    .unwrap()
            })
            .collect();
        assert_eq!(seqs, [1, 2, 3, 4, 5, 6]);

        let reopened = repo.open(&info).unwrap();
        assert_eq!(
            reopened.get_lanes(),
            [
                LanePointer {
                    lane: "main".into(),
                    leaf_id: None
                },
                LanePointer {
                    lane: "thread".into(),
                    leaf_id: Some(entry_id.clone())
                },
            ]
        );
        assert_eq!(reopened.get_name(), Some("Example"));
        assert_eq!(reopened.get_label(&entry_id), Some("checkpoint"));
        assert_eq!(
            reopened
                .find_records(&RecordQuery::default())
                .unwrap()
                .into_iter()
                .map(|record| record.id)
                .collect::<Vec<_>>(),
            ["run"]
        );
        assert_eq!(
            reopened
                .find_open_operations("thread", Some(2))
                .unwrap()
                .into_iter()
                .map(|record| record.id)
                .collect::<Vec<_>>(),
            ["run"]
        );
        assert_eq!(
            reopened
                .get_log(&LogOptions::default())
                .unwrap()
                .into_iter()
                .map(|item| item.seq())
                .collect::<Vec<_>>(),
            [1, 2, 3, 4, 5, 6]
        );
    }

    #[test]
    fn repairs_torn_tail_and_missing_newline() {
        let dir = tempdir().unwrap();
        let repo = JsonlSessionRepo::new(dir.path());
        let cwd = dir.path().to_string_lossy().to_string();
        let mut session = repo
            .create(JsonlCreateOptions {
                id: Some("session".into()),
                cwd,
                parent_session_id: None,
                metadata: None,
            })
            .unwrap();
        let info = session.info.clone();
        session
            .append_custom_entry("note", serde_json::json!({ "value": "kept" }))
            .unwrap();
        let valid_prefix = std::fs::read_to_string(&info.path).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&info.path)
            .unwrap()
            .write_all(br#"{"kind":"entry""#)
            .unwrap();
        let reopened = repo.open(&info).unwrap();
        assert_eq!(
            reopened.find_entries(&EntryQuery::default()).unwrap().len(),
            1
        );
        assert_eq!(std::fs::read_to_string(&info.path).unwrap(), valid_prefix);

        let unterminated = valid_prefix.trim_end().to_string();
        std::fs::write(&info.path, &unterminated).unwrap();
        let repaired = repo.open(&info).unwrap();
        assert_eq!(
            std::fs::read_to_string(&info.path).unwrap(),
            format!("{unterminated}\n")
        );
        assert_eq!(
            repaired.find_entries(&EntryQuery::default()).unwrap().len(),
            1
        );
    }

    #[test]
    fn rejects_unknown_mutation_and_missing_parent_without_rewriting() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("invalid-final-mutation.jsonl");
        let header = serde_json::json!({
            "kind": "header", "version": 4, "id": "invalid-final-mutation",
            "createdAt": 1, "cwd": dir.path().to_string_lossy()
        });
        let body = format!(
            "{}\n{}\n",
            header,
            serde_json::json!({ "kind": "unknown", "seq": 1 })
        );
        std::fs::write(&path, &body).unwrap();
        let info = JsonlSessionInfo {
            id: "invalid-final-mutation".into(),
            created_at: 1,
            parent_session_id: None,
            cwd: dir.path().to_string_lossy().into(),
            path: path.clone(),
            modified_at: 1,
            source_format: 4,
            metadata: None,
        };
        assert_eq!(
            JsonlSessionRepo::new(dir.path())
                .open(&info)
                .unwrap_err()
                .code,
            "invalid_entry"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), body);

        let orphan = dir.path().join("session-missing-parent.jsonl");
        let entry = serde_json::json!({
            "kind": "entry", "type": "custom", "id": "orphan", "customType": "note",
            "parentId": "missing", "seq": 1, "timestamp": 1
        });
        std::fs::write(
            &orphan,
            format!(
                "{}\n{}\n",
                serde_json::json!({
                    "kind": "header", "version": 4, "id": "missing-parent",
                    "createdAt": 1, "cwd": dir.path().to_string_lossy()
                }),
                entry
            ),
        )
        .unwrap();
        let err = JsonlStoredSession::load(&orphan).unwrap_err();
        assert_eq!(err.code, "invalid_entry");
        assert!(err.message.contains("references missing parent missing"));
    }

    #[test]
    fn forks_tree_and_recomputes_stats_on_reopen() {
        let dir = tempdir().unwrap();
        let repo = JsonlSessionRepo::new(dir.path());
        let cwd = dir.path().to_string_lossy().to_string();
        let mut source = repo
            .create(JsonlCreateOptions {
                id: Some("source".into()),
                cwd: cwd.clone(),
                parent_session_id: None,
                metadata: None,
            })
            .unwrap();
        source.append_message("one").unwrap();
        source.append_message("two").unwrap();
        let fork = repo
            .fork(
                &source,
                &ForkOptions {
                    id: Some("fork".into()),
                    ..ForkOptions::default()
                },
                &cwd,
            )
            .unwrap();
        assert_eq!(fork.get_stats().message_count, 2);
        let info = fork.info.clone();
        let mut reopened = repo.open(&info).unwrap();
        assert_eq!(reopened.get_stats().message_count, 2);
        reopened.append_message("three").unwrap();
        assert_eq!(reopened.get_stats().message_count, 3);
        assert_eq!(repo.open(&info).unwrap().get_stats().message_count, 3);

        let mut tree_source = repo
            .create(JsonlCreateOptions {
                id: Some("tree-source".into()),
                cwd: cwd.clone(),
                parent_session_id: None,
                metadata: None,
            })
            .unwrap();
        let root_id = tree_source
            .append_custom_entry("root", Value::Null)
            .unwrap();
        tree_source.create_lane("thread", Some(&root_id)).unwrap();
        let main_id = tree_source
            .append_custom_entry("main", Value::Null)
            .unwrap();
        let thread = tree_source
            .append_entry(custom_entry("thread", "thread", Value::Null), "thread")
            .unwrap();
        tree_source.set_name(Some("Source")).unwrap();
        tree_source.set_label(&thread.id, Some("tip")).unwrap();
        let tree_fork = repo
            .fork(
                &tree_source,
                &ForkOptions {
                    scope: ForkScope::Tree,
                    id: Some("tree-fork".into()),
                    ..ForkOptions::default()
                },
                &cwd,
            )
            .unwrap();
        let imported: Vec<bool> = std::fs::read_to_string(&tree_fork.info.path)
            .unwrap()
            .trim_end()
            .lines()
            .skip(1)
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|value| value["kind"] == "entry")
            .map(|value| value.get("lane").is_some())
            .collect();
        assert_eq!(imported, [false, false, false]);
        let reopened_tree = repo.open(&tree_fork.info).unwrap();
        assert_eq!(
            reopened_tree
                .find_entries(&EntryQuery {
                    order: EntryOrder::OldestFirst,
                    ..EntryQuery::default()
                })
                .unwrap()
                .into_iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            [root_id, main_id, thread.id.clone()]
        );
        assert_eq!(reopened_tree.get_name(), Some("Source"));
        assert_eq!(reopened_tree.get_label(&thread.id), Some("tip"));
        assert!(reopened_tree
            .find_records(&RecordQuery::default())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn exposes_metadata_contract_and_expected_path() {
        let dir = tempdir().unwrap();
        let repo = JsonlSessionRepo::new(dir.path());
        let cwd = dir.path().join("workspace/project");
        let session = repo
            .create(JsonlCreateOptions {
                id: Some("metadata".into()),
                cwd: cwd.to_string_lossy().into(),
                parent_session_id: Some("parent".into()),
                metadata: Some(
                    serde_json::json!({ "owner": "agent", "nested": { "enabled": true } }),
                ),
            })
            .unwrap();
        let info = session.info();
        assert_eq!(info.id, "metadata");
        assert_eq!(info.parent_session_id.as_deref(), Some("parent"));
        assert_eq!(info.cwd, cwd.to_string_lossy());
        assert_eq!(info.source_format, 4);
        assert_eq!(
            info.path,
            expected_session_path(dir.path(), &info.cwd, info.created_at, &info.id)
        );
        assert_eq!(
            info.metadata,
            Some(serde_json::json!({ "owner": "agent", "nested": { "enabled": true } }))
        );
        assert_eq!(
            repo.list(Some(&cwd.to_string_lossy()))
                .unwrap()
                .into_iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            ["metadata"]
        );
        assert!(repo
            .list(Some(&dir.path().join("other/project").to_string_lossy()))
            .unwrap()
            .is_empty());
    }

    fn create_fixture(repo: &JsonlSessionRepo, id: &str, cwd: &str) -> JsonlStoredSession {
        repo.create(JsonlCreateOptions {
            id: Some(id.into()),
            cwd: cwd.into(),
            parent_session_id: None,
            metadata: None,
        })
        .unwrap()
    }

    #[test]
    fn tail_repair_waits_for_an_in_flight_append_instead_of_erasing_it() {
        let dir = tempdir().unwrap();
        let repo = JsonlSessionRepo::new(dir.path());
        let mut writer = create_fixture(&repo, "racing", "/fixture");
        writer.append_message("first").unwrap();
        let path = writer.info.path.clone();
        // Another handle is mid-append: it holds the session lock and has
        // written only part of its line.
        let lock =
            davinci_sys::lock::ExclusiveFileLock::try_acquire(&session_lock_path(&path)).unwrap();
        let line = encode_mutation(
            &writer
                .session
                .plan_entry(user_message_entry("second-id", "second"), "main")
                .unwrap()
                .0,
        );
        let (head, rest) = line.split_at(line.len() / 2);
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(head.as_bytes())
            .unwrap();
        let loader = {
            let path = path.clone();
            std::thread::spawn(move || JsonlStoredSession::load(&path))
        };
        // Give the loader time to see the torn tail; it must wait, not rewrite.
        std::thread::sleep(std::time::Duration::from_millis(300));
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(rest.as_bytes())
            .unwrap();
        drop(lock);
        let loaded = loader.join().unwrap().unwrap();
        assert_eq!(loaded.get_stats().message_count, 2);
        assert!(fs::read_to_string(&path).unwrap().ends_with(&line));
    }

    #[test]
    fn a_tail_held_by_a_live_writer_loads_read_only_and_untouched() {
        let dir = tempdir().unwrap();
        let repo = JsonlSessionRepo::new(dir.path());
        let mut writer = create_fixture(&repo, "held", "/fixture");
        writer.append_message("first").unwrap();
        let path = writer.info.path.clone();
        let _held =
            davinci_sys::lock::ExclusiveFileLock::try_acquire(&session_lock_path(&path)).unwrap();
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(br#"{"kind":"entry""#)
            .unwrap();
        let before = fs::read(&path).unwrap();
        let mut loaded = JsonlStoredSession::load(&path).unwrap();
        assert_eq!(loaded.get_stats().message_count, 1);
        assert_eq!(
            fs::read(&path).unwrap(),
            before,
            "a held tail was rewritten"
        );
        let err = loaded.append_message("refused").unwrap_err();
        assert!(err.message.contains("reopen"), "{}", err.message);
    }

    #[test]
    fn concurrent_forks_with_one_id_create_exactly_one_session() {
        for _ in 0..10 {
            let dir = tempdir().unwrap();
            let repo = std::sync::Arc::new(JsonlSessionRepo::new(dir.path()));
            let mut source = create_fixture(&repo, "source", "/fixture");
            source.append_message("hello").unwrap();
            let source = std::sync::Arc::new(source);
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
            let forks: Vec<_> = (0..2)
                .map(|_| {
                    let (repo, source, barrier) = (repo.clone(), source.clone(), barrier.clone());
                    std::thread::spawn(move || {
                        barrier.wait();
                        repo.fork(
                            &source,
                            &ForkOptions {
                                id: Some("twin".into()),
                                ..ForkOptions::default()
                            },
                            "/fixture",
                        )
                    })
                })
                .collect();
            let results: Vec<_> = forks.into_iter().map(|t| t.join().unwrap()).collect();
            assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
            let err = results.iter().find_map(|r| r.as_ref().err()).unwrap();
            assert_eq!(err.code, "already_exists");
            let twins: Vec<_> = repo
                .list(Some("/fixture"))
                .unwrap()
                .into_iter()
                .filter(|info| info.id == "twin")
                .collect();
            assert_eq!(twins.len(), 1);
            assert_eq!(repo.open(&twins[0]).unwrap().get_stats().message_count, 1);
        }
    }

    #[test]
    fn listing_skips_an_oversized_header_without_buffering_it() {
        let dir = tempdir().unwrap();
        let repo = JsonlSessionRepo::new(dir.path());
        let valid = create_fixture(&repo, "valid", "/fixture");
        let huge = valid.info.path.with_file_name("huge.jsonl");
        fs::write(&huge, vec![b'x'; crate::bounded::MAX_HEADER_LINE_BYTES * 4]).unwrap();
        let ids: Vec<_> = repo
            .list(Some("/fixture"))
            .unwrap()
            .into_iter()
            .map(|info| info.id)
            .collect();
        assert_eq!(ids, ["valid"]);
    }

    #[cfg(unix)]
    #[test]
    fn listing_does_not_follow_symlinked_directories() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let elsewhere = JsonlSessionRepo::new(outside.path());
        let planted = create_fixture(&elsewhere, "planted", "/elsewhere");
        let root = dir.path().join("sessions");
        fs::create_dir(&root).unwrap();
        std::os::unix::fs::symlink(planted.info.path.parent().unwrap(), root.join("--link--"))
            .unwrap();
        let repo = JsonlSessionRepo::new(&root);
        create_fixture(&repo, "own", "/fixture");
        let ids: Vec<_> = repo.list(None).unwrap().into_iter().map(|i| i.id).collect();
        assert_eq!(ids, ["own"]);
        let found: Vec<_> = crate::discover_sessions(&root, None)
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(found, ["own"]);
    }

    #[test]
    fn skips_malformed_headers_when_listing() {
        let dir = tempdir().unwrap();
        let repo = JsonlSessionRepo::new(dir.path());
        let cwd = dir.path().to_string_lossy().to_string();
        repo.create(JsonlCreateOptions {
            id: Some("valid".into()),
            cwd: cwd.clone(),
            parent_session_id: None,
            metadata: None,
        })
        .unwrap();
        let malformed = repo
            .create(JsonlCreateOptions {
                id: Some("malformed-header".into()),
                cwd,
                parent_session_id: None,
                metadata: None,
            })
            .unwrap();
        std::fs::write(&malformed.info.path, "not json\n").unwrap();
        assert_eq!(
            repo.open(&malformed.info).unwrap_err().code,
            "invalid_entry"
        );
        assert_eq!(
            repo.list(Some(&dir.path().to_string_lossy()))
                .unwrap()
                .into_iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            ["valid"]
        );
    }
}
