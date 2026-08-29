//! JSONL session store compatible with TypeScript pi (`~/.pi` and `--session-dir`).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;
use walkdir::WalkDir;

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("is not valid JSON")]
    Syntax,
    #[error("is not a JSON object")]
    NotObject,
    #[error("has invalid {0}")]
    InvalidField(&'static str),
    #[error("is not a header")]
    NotHeader,
    #[error("has unsupported session version")]
    UnsupportedVersion,
    #[error("has both parentSessionId and legacyParentSessionPath")]
    BothParents,
    #[error("has unknown entry type {0}")]
    UnknownEntryType(String),
    #[error("Invalid JSONL v4 session {path}: line {line} {message}")]
    InvalidFile {
        path: String,
        line: usize,
        message: String,
    },
    #[error("{0}")]
    Storage(String),
}

impl SessionError {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Syntax => "syntax",
            _ => "schema",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonlV4Header {
    pub kind: String,
    pub version: u32,
    pub id: String,
    #[serde(rename = "createdAt")]
    pub created_at: i64,
    pub cwd: String,
    #[serde(rename = "parentSessionId", skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(
        rename = "legacyParentSessionPath",
        skip_serializing_if = "Option::is_none"
    )]
    pub legacy_parent_session_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionInfo {
    pub id: String,
    pub path: PathBuf,
    pub cwd: String,
    pub created_at: i64,
    pub modified_at: i64,
    pub source_format: u8,
    pub name: Option<String>,
    pub parent_session_id: Option<String>,
}

pub fn parse_header(line: &str) -> Result<JsonlV4Header, SessionError> {
    let value: Value = serde_json::from_str(line).map_err(|_| SessionError::Syntax)?;
    let obj = value.as_object().ok_or(SessionError::NotObject)?;
    if obj.get("kind").and_then(|v| v.as_str()) != Some("header") {
        return Err(SessionError::NotHeader);
    }
    if obj.get("version").and_then(|v| v.as_u64()) != Some(4) {
        return Err(SessionError::UnsupportedVersion);
    }
    let parent = obj.get("parentSessionId");
    if parent.is_some() && !parent.unwrap().is_string() {
        return Err(SessionError::InvalidField("parentSessionId"));
    }
    let legacy = obj.get("legacyParentSessionPath");
    if legacy.is_some() && !legacy.unwrap().is_string() {
        return Err(SessionError::InvalidField("legacyParentSessionPath"));
    }
    if parent.is_some() && legacy.is_some() {
        return Err(SessionError::BothParents);
    }
    if let Some(meta) = obj.get("metadata") {
        if !meta.is_object() {
            return Err(SessionError::InvalidField("metadata"));
        }
    }
    let id = obj
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or(SessionError::InvalidField("id"))?
        .to_string();
    let created_at = obj
        .get("createdAt")
        .and_then(|v| v.as_i64())
        .ok_or(SessionError::InvalidField("timestamp"))?;
    if created_at < 0 {
        return Err(SessionError::InvalidField("timestamp"));
    }
    let cwd = obj
        .get("cwd")
        .and_then(|v| v.as_str())
        .ok_or(SessionError::InvalidField("cwd"))?
        .to_string();
    Ok(JsonlV4Header {
        kind: "header".into(),
        version: 4,
        id,
        created_at,
        cwd,
        parent_session_id: parent.and_then(|v| v.as_str()).map(str::to_string),
        legacy_parent_session_path: legacy.and_then(|v| v.as_str()).map(str::to_string),
        metadata: obj.get("metadata").cloned(),
    })
}

pub fn encode_header(header: &JsonlV4Header) -> String {
    format!("{}\n", serde_json::to_string(header).expect("header json"))
}

const ENTRY_TYPES: &[&str] = &[
    "message",
    "model_change",
    "thinking_level_change",
    "active_tools_change",
    "compaction",
    "branch_summary",
    "custom",
    "session_info",
];

pub fn parse_entry_line(line: &str) -> Result<Value, SessionError> {
    let value: Value = serde_json::from_str(line).map_err(|_| SessionError::Syntax)?;
    let obj = value.as_object().ok_or(SessionError::NotObject)?;
    if let Some(kind) = obj.get("kind").and_then(|v| v.as_str()) {
        if kind == "header" {
            parse_header(line)?;
            return Ok(value);
        }
    }
    if let Some(entry_type) = obj.get("type").and_then(|v| v.as_str()) {
        if !ENTRY_TYPES.contains(&entry_type) {
            return Err(SessionError::UnknownEntryType(entry_type.to_string()));
        }
    }
    Ok(value)
}

/// v3 files are a stream of entries without a v4 header. First line is typically a message.
pub fn is_v3_session(first_line: &str) -> bool {
    parse_header(first_line).is_err()
        && serde_json::from_str::<Value>(first_line)
            .ok()
            .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_string))
            .is_some()
}

pub fn migrate_v3_to_v4(
    raw: &str,
    path: &Path,
    cwd: &str,
    id: &str,
) -> Result<String, SessionError> {
    let mut out = String::new();
    let header = JsonlV4Header {
        kind: "header".into(),
        version: 4,
        id: id.to_string(),
        created_at: path
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0),
        cwd: cwd.to_string(),
        parent_session_id: None,
        legacy_parent_session_path: None,
        metadata: None,
    };
    out.push_str(&encode_header(&header));
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        parse_entry_line(line)?;
        out.push_str(line);
        out.push('\n');
    }
    Ok(out)
}

/// Encode a cwd as TypeScript does for session directory names.
pub fn encode_cwd_dir(cwd: &str) -> String {
    cwd.replace(['/', '\\', ':'], "--")
}

pub fn default_sessions_root() -> PathBuf {
    if let Ok(dir) = std::env::var("PI_CODING_AGENT_SESSION_DIR") {
        return PathBuf::from(dir);
    }
    dirs_agent_sessions()
}

fn dirs_agent_sessions() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let agent = std::env::var("PI_CODING_AGENT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".pi").join("agent"));
    agent.join("sessions")
}

pub fn discover_sessions(root: &Path, cwd: Option<&str>) -> Result<Vec<SessionInfo>, SessionError> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut sessions = Vec::new();
    let walker = WalkDir::new(root).follow_links(true).max_depth(4);
    for entry in walker {
        let entry = entry.map_err(|e| SessionError::Storage(e.to_string()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        if let Ok(info) = read_session_info(path) {
            if let Some(cwd) = cwd {
                if info.cwd != cwd && !path_matches_cwd(path, cwd) {
                    continue;
                }
            }
            sessions.push(info);
        }
    }
    sessions.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
    Ok(sessions)
}

fn path_matches_cwd(path: &Path, cwd: &str) -> bool {
    path.parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .is_some_and(|name| name == encode_cwd_dir(cwd) || name.contains(&encode_cwd_dir(cwd)))
}

pub fn read_session_info(path: &Path) -> Result<SessionInfo, SessionError> {
    let file = fs::File::open(path).map_err(|e| SessionError::Storage(e.to_string()))?;
    let mut lines = BufReader::new(file).lines();
    let first = lines
        .next()
        .ok_or_else(|| SessionError::Storage("empty session".into()))?
        .map_err(|e| SessionError::Storage(e.to_string()))?;
    let modified_at = path
        .metadata()
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    if let Ok(header) = parse_header(&first) {
        let mut name = None;
        for line in lines.map_while(Result::ok) {
            if let Ok(value) = serde_json::from_str::<Value>(&line) {
                if value.get("type").and_then(|v| v.as_str()) == Some("session_info") {
                    name = value
                        .get("name")
                        .and_then(|v| v.as_str())
                        .map(str::to_string);
                }
            }
        }
        return Ok(SessionInfo {
            id: header.id,
            path: path.to_path_buf(),
            cwd: header.cwd,
            created_at: header.created_at,
            modified_at,
            source_format: 4,
            name,
            parent_session_id: header.parent_session_id,
        });
    }
    let id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
    Ok(SessionInfo {
        id,
        path: path.to_path_buf(),
        cwd: String::new(),
        created_at: modified_at,
        modified_at,
        source_format: 3,
        name: None,
        parent_session_id: None,
    })
}

pub fn continue_latest(
    root: &Path,
    cwd: Option<&str>,
) -> Result<Option<SessionInfo>, SessionError> {
    Ok(discover_sessions(root, cwd)?.into_iter().next())
}

pub fn resume_by_id_or_path(
    root: &Path,
    query: &str,
    cwd: Option<&str>,
) -> Result<Option<SessionInfo>, SessionError> {
    let query_path = PathBuf::from(query);
    if query_path.exists() {
        return read_session_info(&query_path).map(Some);
    }
    let sessions = discover_sessions(root, cwd)?;
    Ok(sessions.into_iter().find(|s| {
        s.id == query
            || s.id.starts_with(query)
            || s.path.to_string_lossy().contains(query)
            || s.name.as_deref() == Some(query)
    }))
}

pub fn create_session(
    root: &Path,
    cwd: &str,
    id: Option<&str>,
) -> Result<SessionInfo, SessionError> {
    let id = id
        .map(str::to_string)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let dir = root.join(encode_cwd_dir(cwd));
    fs::create_dir_all(&dir).map_err(|e| SessionError::Storage(e.to_string()))?;
    let path = dir.join(format!("{id}.jsonl"));
    let created_at = now_ms();
    let header = JsonlV4Header {
        kind: "header".into(),
        version: 4,
        id: id.clone(),
        created_at,
        cwd: cwd.to_string(),
        parent_session_id: None,
        legacy_parent_session_path: None,
        metadata: None,
    };
    fs::write(&path, encode_header(&header)).map_err(|e| SessionError::Storage(e.to_string()))?;
    Ok(SessionInfo {
        id,
        path,
        cwd: cwd.to_string(),
        created_at,
        modified_at: created_at,
        source_format: 4,
        name: None,
        parent_session_id: None,
    })
}

pub fn append_entry(path: &Path, entry: &Value) -> Result<(), SessionError> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| SessionError::Storage(e.to_string()))?;
    writeln!(file, "{entry}").map_err(|e| SessionError::Storage(e.to_string()))
}

pub fn fork_session(
    root: &Path,
    source: &SessionInfo,
    cwd: &str,
) -> Result<SessionInfo, SessionError> {
    let mut dest = create_session(root, cwd, None)?;
    dest.parent_session_id = Some(source.id.clone());
    let raw = fs::read_to_string(&source.path).map_err(|e| SessionError::Storage(e.to_string()))?;
    let mut rewritten = String::new();
    for (i, line) in raw.lines().enumerate() {
        if i == 0 {
            if let Ok(mut header) = parse_header(line) {
                header.id = dest.id.clone();
                header.parent_session_id = Some(source.id.clone());
                header.cwd = cwd.to_string();
                rewritten.push_str(&encode_header(&header));
                continue;
            }
        }
        rewritten.push_str(line);
        rewritten.push('\n');
    }
    fs::write(&dest.path, rewritten).map_err(|e| SessionError::Storage(e.to_string()))?;
    Ok(dest)
}

pub fn clone_session(
    root: &Path,
    source: &SessionInfo,
    cwd: &str,
) -> Result<SessionInfo, SessionError> {
    fork_session(root, source, cwd)
}

pub fn append_session_name(path: &Path, name: &str) -> Result<(), SessionError> {
    append_entry(
        path,
        &json!({
            "type": "session_info",
            "name": name,
            "timestamp": now_ms(),
        }),
    )
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn read_entries(path: &Path) -> Result<Vec<Value>, SessionError> {
    let raw = fs::read_to_string(path).map_err(|e| SessionError::Storage(e.to_string()))?;
    let mut entries = Vec::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let value = parse_entry_line(line)?;
        if value.get("kind").and_then(|v| v.as_str()) == Some("header") {
            continue;
        }
        entries.push(value);
    }
    Ok(entries)
}

pub fn last_assistant_text(path: &Path) -> Result<Option<String>, SessionError> {
    let mut last = None;
    for entry in read_entries(path)? {
        if entry.get("type").and_then(|v| v.as_str()) != Some("message") {
            continue;
        }
        let role = entry
            .get("role")
            .or_else(|| entry.get("message").and_then(|m| m.get("role")))
            .and_then(|v| v.as_str());
        if role != Some("assistant") {
            continue;
        }
        let text = entry
            .get("content")
            .or_else(|| entry.get("text"))
            .or_else(|| entry.get("message").and_then(|m| m.get("content")))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        if text.is_some() {
            last = text;
        }
    }
    Ok(last)
}

pub fn session_tree(path: &Path) -> Result<Vec<Value>, SessionError> {
    let entries = read_entries(path)?;
    Ok(entries
        .into_iter()
        .map(|entry| {
            json!({
                "id": entry.get("id").cloned().unwrap_or(json!(null)),
                "parentId": entry.get("parentId").cloned().unwrap_or(json!(null)),
                "type": entry.get("type").cloned().unwrap_or(json!("message")),
                "role": entry.get("role").cloned().unwrap_or(json!(null)),
                "label": entry.get("content").or_else(|| entry.get("text")).cloned().unwrap_or(json!("")),
            })
        })
        .collect())
}

pub fn session_stats(path: &Path) -> Result<Value, SessionError> {
    let entries = read_entries(path)?;
    let mut message_count = 0u64;
    let mut user = 0u64;
    let mut assistant = 0u64;
    for entry in &entries {
        if entry.get("type").and_then(|v| v.as_str()) == Some("message") {
            message_count += 1;
            match entry.get("role").and_then(|v| v.as_str()) {
                Some("user") => user += 1,
                Some("assistant") => assistant += 1,
                _ => {}
            }
        }
    }
    Ok(json!({
        "messageCount": message_count,
        "userMessages": user,
        "assistantMessages": assistant,
        "entryCount": entries.len(),
    }))
}

pub fn fork_from_entry(
    root: &Path,
    source: &SessionInfo,
    cwd: &str,
    entry_id: &str,
) -> Result<SessionInfo, SessionError> {
    let mut dest = create_session(root, cwd, None)?;
    dest.parent_session_id = Some(source.id.clone());
    let raw = fs::read_to_string(&source.path).map_err(|e| SessionError::Storage(e.to_string()))?;
    let mut rewritten = String::new();
    let mut found = false;
    for (i, line) in raw.lines().enumerate() {
        if i == 0 {
            if let Ok(mut header) = parse_header(line) {
                header.id = dest.id.clone();
                header.parent_session_id = Some(source.id.clone());
                header.cwd = cwd.to_string();
                rewritten.push_str(&encode_header(&header));
                continue;
            }
        }
        rewritten.push_str(line);
        rewritten.push('\n');
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            if value.get("id").and_then(|v| v.as_str()) == Some(entry_id) {
                found = true;
                break;
            }
        }
    }
    if !found {
        return Err(SessionError::Storage(format!(
            "entry {entry_id} not found in session"
        )));
    }
    fs::write(&dest.path, rewritten).map_err(|e| SessionError::Storage(e.to_string()))?;
    Ok(dest)
}

pub fn fork_messages(path: &Path) -> Result<Vec<Value>, SessionError> {
    Ok(read_entries(path)?
        .into_iter()
        .filter(|e| {
            e.get("type").and_then(|v| v.as_str()) == Some("message")
                && e.get("role").and_then(|v| v.as_str()) == Some("user")
        })
        .map(|e| {
            json!({
                "entryId": e.get("id").cloned().unwrap_or(json!(null)),
                "text": e.get("content").or_else(|| e.get("text")).cloned().unwrap_or(json!("")),
            })
        })
        .collect())
}

pub fn leaf_id(path: &Path) -> Result<Option<String>, SessionError> {
    Ok(read_entries(path)?
        .into_iter()
        .rev()
        .find_map(|e| e.get("id").and_then(|v| v.as_str()).map(str::to_string)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn header_roundtrip_matches_ts_errors() {
        assert!(matches!(
            parse_header("not-json"),
            Err(SessionError::Syntax)
        ));
        assert!(matches!(parse_header("[]"), Err(SessionError::NotObject)));
        assert!(matches!(
            parse_header(r#"{"kind":"entry","version":4}"#),
            Err(SessionError::NotHeader)
        ));
        assert!(matches!(
            parse_header(r#"{"kind":"header","version":3,"id":"a","createdAt":1,"cwd":"/"}"#),
            Err(SessionError::UnsupportedVersion)
        ));
        let header =
            parse_header(r#"{"kind":"header","version":4,"id":"abc","createdAt":1,"cwd":"/tmp"}"#)
                .unwrap();
        assert_eq!(header.id, "abc");
        assert!(encode_header(&header).contains("\"version\":4"));
    }

    #[test]
    fn continue_resume_fork_discovery() {
        let dir = tempdir().unwrap();
        let created = create_session(dir.path(), "/proj", Some("aaaa-1111")).unwrap();
        append_session_name(&created.path, "first").unwrap();
        let latest = continue_latest(dir.path(), Some("/proj")).unwrap().unwrap();
        assert_eq!(latest.id, "aaaa-1111");
        let resumed = resume_by_id_or_path(dir.path(), "aaaa", Some("/proj"))
            .unwrap()
            .unwrap();
        assert_eq!(resumed.id, "aaaa-1111");
        let forked = fork_session(dir.path(), &resumed, "/proj").unwrap();
        assert_eq!(forked.parent_session_id.as_deref(), Some("aaaa-1111"));
        assert_ne!(forked.id, resumed.id);
    }

    #[test]
    fn v3_migrate_adds_v4_header() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("old.jsonl");
        fs::write(&path, "{\"type\":\"message\",\"id\":\"m1\"}\n").unwrap();
        let migrated = migrate_v3_to_v4(
            &fs::read_to_string(&path).unwrap(),
            &path,
            "/old",
            "migrated",
        )
        .unwrap();
        assert!(migrated.starts_with("{\"kind\":\"header\"") || migrated.contains("\"version\":4"));
        parse_header(migrated.lines().next().unwrap()).unwrap();
    }

    #[test]
    fn entries_tree_fork_from_id() {
        let dir = tempdir().unwrap();
        let created = create_session(dir.path(), "/proj", Some("bbbb-2222")).unwrap();
        append_entry(
            &created.path,
            &json!({"type":"message","id":"m1","role":"user","content":"hello"}),
        )
        .unwrap();
        append_entry(
            &created.path,
            &json!({"type":"message","id":"m2","role":"assistant","content":"world"}),
        )
        .unwrap();
        assert_eq!(
            last_assistant_text(&created.path).unwrap().as_deref(),
            Some("world")
        );
        assert_eq!(session_tree(&created.path).unwrap().len(), 2);
        assert_eq!(session_stats(&created.path).unwrap()["messageCount"], 2);
        let forked = fork_from_entry(dir.path(), &created, "/proj", "m1").unwrap();
        assert_eq!(read_entries(&forked.path).unwrap().len(), 1);
        assert_eq!(fork_messages(&created.path).unwrap().len(), 1);
        assert_eq!(leaf_id(&created.path).unwrap().as_deref(), Some("m2"));
    }
}
