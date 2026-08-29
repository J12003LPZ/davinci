use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::codec::parse_header;
use crate::errors::SessionError;
use crate::JsonlSession;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    pub id: String,
    pub path: PathBuf,
    pub cwd: String,
    pub created_at: u64,
    pub modified_at: u64,
    pub name: Option<String>,
    pub parent_session_id: Option<String>,
    pub source_format: u8,
}

pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }
    if path == "~" {
        if let Some(home) = home_dir() {
            return home;
        }
    }
    PathBuf::from(path)
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub fn default_agent_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("PI_CODING_AGENT_DIR") {
        return expand_tilde(&dir);
    }
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".pi")
        .join("agent")
}

pub fn default_session_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("PI_CODING_AGENT_SESSION_DIR") {
        return expand_tilde(&dir);
    }
    default_agent_dir().join("sessions")
}

pub fn resolve_session_dir(explicit: Option<&str>) -> PathBuf {
    explicit
        .map(expand_tilde)
        .unwrap_or_else(default_session_dir)
}

pub fn encode_cwd_component(cwd: &str) -> String {
    let normalized = cwd.replace('\\', "/");
    let trimmed = normalized.trim_end_matches('/');
    if trimmed.starts_with('/') {
        format!("--{}", trimmed.trim_start_matches('/').replace('/', "--"))
    } else {
        trimmed.replace('/', "--")
    }
}

pub fn cwd_encoded_dir(sessions_root: &Path, cwd: &str) -> PathBuf {
    sessions_root.join(encode_cwd_component(cwd))
}

fn modified_at(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn summarize_file(path: &Path) -> Option<SessionSummary> {
    let first = fs::read_to_string(path).ok()?;
    let first_line = first.lines().next()?;
    if let Ok(header) = parse_header(first_line) {
        return Some(crate::codec::metadata_from_header(
            &header,
            path,
            modified_at(path),
        ));
    }
    let session = JsonlSession::open(path).ok()?;
    Some(SessionSummary {
        id: session.header.id,
        path: path.to_path_buf(),
        cwd: session.header.cwd,
        created_at: session.header.created_at,
        modified_at: modified_at(path),
        name: session
            .header
            .metadata
            .as_ref()
            .and_then(|value| value.get("name"))
            .and_then(|value| value.as_str())
            .map(str::to_string),
        parent_session_id: session.header.parent_session_id,
        source_format: 3,
    })
}

pub fn discover_sessions(
    session_dir: &Path,
    cwd: Option<&str>,
) -> Result<Vec<SessionSummary>, SessionError> {
    let mut sessions = Vec::new();
    if !session_dir.exists() {
        return Ok(sessions);
    }
    let scan_roots: Vec<PathBuf> = if let Some(cwd) = cwd {
        vec![cwd_encoded_dir(session_dir, cwd)]
    } else {
        fs::read_dir(session_dir)
            .map_err(|err| {
                SessionError::storage(format!("Unable to list session directory: {err}"))
            })?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| path.is_dir())
            .collect()
    };
    for root in scan_roots {
        if !root.is_dir() {
            continue;
        }
        let entries = fs::read_dir(&root).map_err(|err| {
            SessionError::storage(format!("Unable to list session directory: {err}"))
        })?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                if let Some(summary) = summarize_file(&path) {
                    sessions.push(summary);
                }
            }
        }
    }
    sessions.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
    Ok(sessions)
}

pub fn latest_session(
    session_dir: &Path,
    cwd: Option<&str>,
) -> Result<Option<SessionSummary>, SessionError> {
    Ok(discover_sessions(session_dir, cwd)?.into_iter().next())
}

pub fn resolve_session_ref(
    session_dir: &Path,
    cwd: Option<&str>,
    reference: &str,
) -> Result<SessionSummary, SessionError> {
    let expanded = expand_tilde(reference);
    if expanded.exists() {
        return summarize_file(&expanded).ok_or_else(|| {
            SessionError::not_found(format!("Session file not found: {reference}"))
        });
    }
    let sessions = discover_sessions(session_dir, cwd)?;
    if let Some(exact) = sessions.iter().find(|s| s.id == reference) {
        return Ok(exact.clone());
    }
    let matches: Vec<_> = sessions
        .iter()
        .filter(|s| {
            s.id.starts_with(reference)
                || s.path.file_stem().and_then(|s| s.to_str()) == Some(reference)
        })
        .cloned()
        .collect();
    match matches.len() {
        1 => Ok(matches.into_iter().next().unwrap()),
        0 => Err(SessionError::not_found(format!(
            "Session not found: {reference}"
        ))),
        _ => Err(SessionError::invalid_entry(format!(
            "Ambiguous session id prefix: {reference}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn cwd_encoding_matches_ts_style() {
        assert_eq!(
            encode_cwd_component("/home/user/proj"),
            "--home--user--proj"
        );
        assert_eq!(
            cwd_encoded_dir(Path::new("/tmp/sessions"), "/tmp/work").as_os_str(),
            Path::new("/tmp/sessions/--tmp--work").as_os_str()
        );
    }

    #[test]
    fn discover_continue_and_partial_id() {
        let dir = tempdir().unwrap();
        let mut first = JsonlSession::create(dir.path(), "/tmp/work", Some("one")).unwrap();
        first
            .append_entry(crate::SessionEntry::message(
                "user",
                serde_json::json!([{"type":"text","text":"a"}]),
            ))
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let second = JsonlSession::create(dir.path(), "/tmp/work", Some("two")).unwrap();
        let latest = latest_session(dir.path(), Some("/tmp/work"))
            .unwrap()
            .unwrap();
        assert_eq!(latest.id, second.header.id);
        let prefix = &first.header.id[..8];
        let resolved = resolve_session_ref(dir.path(), Some("/tmp/work"), prefix).unwrap();
        assert_eq!(resolved.id, first.header.id);
    }
}
