use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::bounded::{read_line_capped, Line, MAX_HEADER_LINE_BYTES};
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
    /// Concatenated user/assistant text matching TS `SessionInfo.allMessagesText`.
    /// Bounded: past [`DIGEST_HEAD_BYTES`] + [`DIGEST_TAIL_BYTES`] the middle
    /// is replaced by ` … `, so a listing of many long sessions holds a
    /// fixed amount of text per session, keeping the opening turns and the
    /// latest ones.
    pub all_messages_text: String,
    /// Byte length of the full digest before it was bounded, for estimates
    /// such as a session's token count.
    pub messages_text_bytes: usize,
    /// How many message entries the file holds — the turn count a listing
    /// shows without re-reading the file.
    pub message_count: usize,
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

/// TS `os.homedir()`: libuv reads `USERPROFILE` on Windows and `HOME` on
/// POSIX. `HOME` stays as a Windows fallback for MSYS/Git Bash shells.
pub fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    if let Some(profile) = std::env::var_os("USERPROFILE").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(profile));
    }
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub fn default_agent_dir() -> PathBuf {
    if let Ok(dir) =
        std::env::var("DAVINCI_CODING_AGENT_DIR").or_else(|_| std::env::var("PI_CODING_AGENT_DIR"))
    {
        return expand_tilde(&dir);
    }
    let home = home_dir().unwrap_or_else(|| PathBuf::from("."));
    let davinci_dir = home.join(".davinci").join("agent");
    if davinci_dir.exists() {
        return davinci_dir;
    }
    let pi_dir = home.join(".pi").join("agent");
    if pi_dir.exists() {
        return pi_dir;
    }
    davinci_dir
}

pub fn default_session_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("DAVINCI_CODING_AGENT_SESSION_DIR")
        .or_else(|_| std::env::var("PI_CODING_AGENT_SESSION_DIR"))
    {
        return expand_tilde(&dir);
    }
    default_agent_dir().join("sessions")
}

pub fn resolve_session_dir(explicit: Option<&str>) -> PathBuf {
    resolve_session_dir_from(explicit, None)
}

/// TS session dir order: `--session-dir`, `DAVINCI/PI_CODING_AGENT_SESSION_DIR`, `settings.sessionDir`, default.
pub fn resolve_session_dir_from(explicit: Option<&str>, settings_dir: Option<&str>) -> PathBuf {
    if let Some(dir) = explicit.map(str::trim).filter(|dir| !dir.is_empty()) {
        return expand_tilde(dir);
    }
    if let Ok(dir) = std::env::var("DAVINCI_CODING_AGENT_SESSION_DIR")
        .or_else(|_| std::env::var("PI_CODING_AGENT_SESSION_DIR"))
    {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return expand_tilde(trimmed);
        }
    }
    if let Some(dir) = settings_dir.map(str::trim).filter(|dir| !dir.is_empty()) {
        return expand_tilde(dir);
    }
    default_agent_dir().join("sessions")
}

/// TS `getDefaultSessionDirPath` safePath: strip ONE leading `/` or `\`,
/// replace every `/`, `\`, `:` with `-`, wrap in `--..--`. Replacing `:`
/// matters beyond parity: keeping `C:` produced a drive-relative component
/// that made `Path::join` discard the sessions root on Windows.
pub fn encode_cwd_component(cwd: &str) -> String {
    let stripped = cwd.strip_prefix(['/', '\\']).unwrap_or(cwd);
    let replaced = stripped.replace(['/', '\\', ':'], "-");
    format!("--{replaced}--")
}

/// Directory name produced by Rust builds that predate the TS-format
/// alignment above; still scanned so their sessions stay discoverable.
fn legacy_encode_cwd_component(cwd: &str) -> String {
    let normalized = cwd.replace('\\', "/");
    let trimmed = normalized.trim_end_matches('/');
    if trimmed.starts_with('/') {
        format!("--{}", trimmed.trim_start_matches('/').replace('/', "--"))
    } else {
        trimmed.replace('/', "--")
    }
}

/// Whether a session recorded under `recorded` belongs to the working
/// directory `requested`.
///
/// The encoded directory name is lossy (`/a/b-c` and `/a/b/c` both become
/// `--a-b-c--`), and renaming the scheme would orphan every existing session
/// directory, so the name only narrows the scan. The header's own `cwd` is
/// the authority. Separator style and trailing separators are ignored (older
/// builds recorded either), as is case on Windows, where `C:\Dev` and
/// `c:\dev` are one directory. A session with no recorded `cwd` cannot be
/// disproved and stays visible.
pub fn same_cwd(recorded: &str, requested: &str) -> bool {
    fn canon(path: &str) -> String {
        let unified = path.replace('\\', "/");
        let trimmed = unified.trim_end_matches('/');
        let trimmed = if trimmed.is_empty() && unified.starts_with('/') {
            "/"
        } else {
            trimmed
        };
        if cfg!(windows) {
            trimmed.to_lowercase()
        } else {
            trimmed.to_string()
        }
    }
    recorded.is_empty() || canon(recorded) == canon(requested)
}

fn keep_for_cwd(summary: &SessionSummary, cwd: Option<&str>) -> bool {
    cwd.is_none_or(|requested| same_cwd(&summary.cwd, requested))
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
    // One streamed pass serves both the header and the message-text digest;
    // reading the file for the header and re-opening it through
    // `JsonlSession::open` for the text doubled I/O and parsing across every
    // session in a listing. Only one line is resident at a time, so a listing
    // never holds a whole transcript besides the digest it returns.
    let mut reader = BufReader::new(fs::File::open(path).ok()?);
    let mut first_line = Vec::new();
    // A header too long to be one is not a session, and must not be
    // buffered whole (or handed to the full legacy loader) to find that out.
    if read_line_capped(&mut reader, MAX_HEADER_LINE_BYTES, &mut first_line).ok()? != Line::Complete
    {
        return None;
    }
    let first_line = String::from_utf8(first_line).ok()?;
    if let Ok(header) = parse_header(first_line.trim_end()) {
        let mut summary = crate::codec::metadata_from_header(&header, path, modified_at(path));
        let (text, count, bytes) = messages_text_from_reader(reader).ok()?;
        summary.all_messages_text = text;
        summary.messages_text_bytes = bytes;
        summary.message_count = count;
        return Some(summary);
    }
    // Legacy v3 file: a full open performs the migration.
    let session = JsonlSession::open(path).ok()?;
    let (legacy_text, legacy_count, legacy_bytes) = messages_text_from_entries(&session.entries);
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
        all_messages_text: legacy_text,
        messages_text_bytes: legacy_bytes,
        message_count: legacy_count,
    })
}

fn summarize_header(path: &Path) -> Option<SessionSummary> {
    let first_line = crate::bounded::read_header_line(path)?;
    if let Ok(header) = parse_header(first_line.trim_end()) {
        return Some(crate::codec::metadata_from_header(
            &header,
            path,
            modified_at(path),
        ));
    }
    // Legacy v3 has no v4 header, so preserve compatibility by migrating it
    // through the existing loader.
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
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        parent_session_id: session.header.parent_session_id,
        source_format: 3,
        all_messages_text: String::new(),
        messages_text_bytes: 0,
        message_count: 0,
    })
}

fn is_session_jsonl(path: &Path) -> bool {
    if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
        return false;
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    !name.ends_with(".runtime.jsonl")
}

fn extract_text_content(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| {
                (block.get("type").and_then(|value| value.as_str()) == Some("text"))
                    .then(|| block.get("text").and_then(|value| value.as_str()))
                    .flatten()
                    .map(str::to_string)
            })
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

fn message_text(message: &serde_json::Value) -> Option<String> {
    let role = message
        .get("role")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    if role != "user" && role != "assistant" {
        return None;
    }
    let text = extract_text_content(message.get("content")?);
    (!text.is_empty()).then_some(text)
}

/// Bytes of a session's message text kept from its start, and from its end.
pub const DIGEST_HEAD_BYTES: usize = 32 * 1024;
pub const DIGEST_TAIL_BYTES: usize = 32 * 1024;
/// A record longer than this is skipped by the digest rather than buffered.
/// Message text that large is tool output pasted as a turn, not prose a
/// picker needs to search.
const MAX_DIGEST_LINE_BYTES: usize = 8 * 1024 * 1024;

/// The space-joined message text of one session, bounded to its head and
/// tail as it streams, so memory stays flat whatever the transcript's size.
#[derive(Default)]
struct Digest {
    text: String,
    /// Where the head ends once the middle has been dropped.
    head_end: Option<usize>,
    bytes: usize,
}

impl Digest {
    fn push(&mut self, part: &str) {
        if self.bytes > 0 {
            self.text.push(' ');
            self.bytes += 1;
        }
        self.text.push_str(part);
        self.bytes += part.len();
        // Compacting only once the slack reaches a whole tail keeps the
        // copying amortized; `finish` trims the remainder.
        if self.text.len() > DIGEST_HEAD_BYTES + 2 * DIGEST_TAIL_BYTES {
            self.compact();
        }
    }

    fn compact(&mut self) {
        let head_end = *self
            .head_end
            .get_or_insert_with(|| floor_char_boundary(&self.text, DIGEST_HEAD_BYTES));
        let tail_start = ceil_char_boundary(
            &self.text,
            self.text.len().saturating_sub(DIGEST_TAIL_BYTES),
        )
        .max(head_end);
        self.text.replace_range(head_end..tail_start, "");
    }

    fn finish(mut self) -> (String, usize) {
        if self.head_end.is_some() || self.text.len() > DIGEST_HEAD_BYTES + DIGEST_TAIL_BYTES {
            self.compact();
        }
        let mut text = self.text;
        if let Some(head_end) = self.head_end {
            text.insert_str(head_end, " … ");
        }
        (text, self.bytes)
    }
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(text: &str, mut index: usize) -> usize {
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

/// Digest the remaining lines of a v4 file without building a full session.
/// Returns the bounded text, the message count, and the unbounded text size.
fn messages_text_from_reader(mut reader: impl BufRead) -> std::io::Result<(String, usize, usize)> {
    let mut count = 0usize;
    let mut digest = Digest::default();
    let mut raw = Vec::new();
    loop {
        match read_line_capped(&mut reader, MAX_DIGEST_LINE_BYTES, &mut raw)? {
            Line::Eof => break,
            Line::TooLong => {
                crate::bounded::skip_line(&mut reader)?;
                continue;
            }
            Line::Complete => {}
        }
        let Ok(line) = std::str::from_utf8(&raw) else {
            continue;
        };
        let line = line.trim();
        // Cheap pre-filter: only message entries can contribute text.
        if line.is_empty() || !line.contains("\"message\"") {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("kind").and_then(|v| v.as_str()) != Some("entry")
            || value.get("type").and_then(|v| v.as_str()) != Some("message")
        {
            continue;
        }
        count += 1;
        if let Some(part) = value.get("message").and_then(message_text) {
            digest.push(&part);
        }
    }
    let (text, bytes) = digest.finish();
    Ok((text, count, bytes))
}

fn messages_text_from_entries(entries: &[crate::SessionEntry]) -> (String, usize, usize) {
    let mut count = 0usize;
    let mut digest = Digest::default();
    for entry in entries {
        if entry.entry_type != "message" {
            continue;
        }
        count += 1;
        if let Some(text) = entry.message.as_ref().and_then(message_text) {
            digest.push(&text);
        }
    }
    let (text, bytes) = digest.finish();
    (text, count, bytes)
}

/// Session directories directly under `root`. Symlinks are never followed:
/// a link planted in the sessions root must not pull another directory's
/// JSONL files into a listing.
pub(crate) fn session_subdirectories(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    Ok(fs::read_dir(root)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .collect())
}

/// Whether a directory entry is a regular session file (not a symlink).
fn is_session_file(entry: &fs::DirEntry) -> bool {
    entry.file_type().is_ok_and(|kind| kind.is_file()) && is_session_jsonl(&entry.path())
}

/// Every directory a session for `cwd` may live in: the current encoding
/// (where every session is now written), the pre-alignment Rust encoding,
/// and the name `JsonlSessionRepo` used in older builds for sessions created
/// with an explicit id. That name strips every leading separator rather
/// than one, which differs for UNC
/// (`\\server\share`) and `//host` paths. Headers are still matched
/// against `cwd`, so an extra root never admits another directory's session.
pub(crate) fn cwd_scan_roots(session_dir: &Path, cwd: &str) -> Vec<PathBuf> {
    let mut roots = vec![cwd_encoded_dir(session_dir, cwd)];
    for candidate in [
        session_dir.join(legacy_encode_cwd_component(cwd)),
        session_dir.join(crate::jsonl_repo::jsonl_session_directory_name(cwd)),
    ] {
        if !roots.contains(&candidate) {
            roots.push(candidate);
        }
    }
    roots
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
        cwd_scan_roots(session_dir, cwd)
    } else {
        let mut roots = session_subdirectories(session_dir).map_err(|err| {
            SessionError::storage(format!("Unable to list session directory: {err}"))
        })?;
        roots.push(session_dir.to_path_buf());
        roots
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
            if is_session_file(&entry) {
                if let Some(summary) = summarize_file(&path) {
                    if keep_for_cwd(&summary, cwd) {
                        sessions.push(summary);
                    }
                }
            }
        }
    }
    sessions.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
    sessions.dedup_by(|a, b| a.path == b.path);
    Ok(sessions)
}

/// List sessions using only the v4 header line. This is intended for callers
/// that need metadata but not transcript search text.
pub fn discover_session_headers(
    session_dir: &Path,
    cwd: Option<&str>,
) -> Result<Vec<SessionSummary>, SessionError> {
    let mut sessions = Vec::new();
    if !session_dir.exists() {
        return Ok(sessions);
    }
    let scan_roots: Vec<PathBuf> = if let Some(cwd) = cwd {
        cwd_scan_roots(session_dir, cwd)
    } else {
        let mut roots = session_subdirectories(session_dir).map_err(|err| {
            SessionError::storage(format!("Unable to list session directory: {err}"))
        })?;
        roots.push(session_dir.to_path_buf());
        roots
    };
    for root in scan_roots {
        if !root.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&root)
            .map_err(|err| {
                SessionError::storage(format!("Unable to list session directory: {err}"))
            })?
            .flatten()
        {
            let path = entry.path();
            if is_session_file(&entry) {
                if let Some(summary) = summarize_header(&path) {
                    if keep_for_cwd(&summary, cwd) {
                        sessions.push(summary);
                    }
                }
            }
        }
    }
    sessions.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
    sessions.dedup_by(|a, b| a.path == b.path);
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
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn sessions_created_with_an_explicit_id_are_found_from_their_cwd() {
        // `--session-id` creates through JsonlSessionRepo, whose directory
        // name strips every leading separator; discovery strips one. They
        // differ for UNC and `//host` cwds.
        let dir = tempdir().unwrap();
        for (index, cwd) in [
            "/home/u/proj",
            r"C:\Users\u\proj",
            r"\\server\share\proj",
            "//server/share/proj",
        ]
        .into_iter()
        .enumerate()
        {
            let id = format!("explicit{index}");
            let created = crate::JsonlSessionRepo::new(dir.path())
                .create(crate::JsonlCreateOptions {
                    id: Some(id.clone()),
                    cwd: cwd.into(),
                    parent_session_id: None,
                    metadata: None,
                })
                .unwrap();
            let found = resolve_session_ref(dir.path(), Some(cwd), &id)
                .unwrap_or_else(|err| panic!("{cwd}: {err}"));
            assert_eq!(found.path, created.info.path, "{cwd}");
            assert!(
                discover_session_headers(dir.path(), Some(cwd))
                    .unwrap()
                    .iter()
                    .any(|session| session.id == id),
                "{cwd}"
            );
        }
        // A session from another cwd sharing a directory name stays hidden.
        assert!(resolve_session_ref(dir.path(), Some(r"\\other\share\proj"), "explicit2").is_err());
    }

    #[test]
    fn explicit_id_sessions_use_the_discovery_directory_and_still_see_legacy_ones() {
        let dir = tempdir().unwrap();
        let repo = crate::JsonlSessionRepo::new(dir.path());
        let cwd = r"\\server\share\proj";
        let create = |id: &str| {
            repo.create(crate::JsonlCreateOptions {
                id: Some(id.into()),
                cwd: cwd.into(),
                parent_session_id: None,
                metadata: None,
            })
        };
        // One encoder for new sessions: the name discovery derives.
        let created = create("fresh").unwrap();
        assert_eq!(
            created.info.path.parent().unwrap(),
            cwd_encoded_dir(dir.path(), cwd)
        );

        // A session an older build wrote under the repo's legacy name.
        let legacy = dir
            .path()
            .join(crate::jsonl_repo::jsonl_session_directory_name(cwd));
        assert_ne!(legacy, cwd_encoded_dir(dir.path(), cwd));
        fs::create_dir_all(&legacy).unwrap();
        let moved = legacy.join(
            created
                .info
                .path
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .replace("fresh", "old"),
        );
        fs::write(
            &moved,
            fs::read_to_string(&created.info.path)
                .unwrap()
                .replace("\"fresh\"", "\"old\""),
        )
        .unwrap();
        // Its id cannot be created again, and listing by cwd finds it.
        assert!(create("old").is_err());
        let listed = repo.list(Some(cwd)).unwrap();
        assert!(listed.iter().any(|info| info.id == "old"), "{listed:?}");
        assert!(listed.iter().any(|info| info.id == "fresh"), "{listed:?}");
    }

    #[test]
    fn cwd_encoding_matches_ts_style() {
        assert_eq!(
            encode_cwd_component("/home/user/proj"),
            "--home-user-proj--"
        );
        assert_eq!(
            encode_cwd_component("C:\\Users\\sergi\\Desktop\\pi-rust"),
            "--C--Users-sergi-Desktop-pi-rust--"
        );
        // Build the expectation with join: the platform separator between root
        // and encoded component matches TS `path.join` behavior on each OS.
        assert_eq!(
            cwd_encoded_dir(Path::new("/tmp/sessions"), "/tmp/work"),
            Path::new("/tmp/sessions").join("--tmp-work--")
        );
    }

    fn session_in(root: &Path, cwd: &str, name: &str) -> String {
        JsonlSession::create(root, cwd, Some(name))
            .unwrap()
            .header
            .id
    }

    #[test]
    fn distinct_cwds_that_encode_to_one_directory_stay_separate() {
        // `/home/u/my-app` and `/home/u/my/app` share `--home-u-my-app--`.
        let dir = tempdir().unwrap();
        assert_eq!(
            encode_cwd_component("/home/u/my-app"),
            encode_cwd_component("/home/u/my/app")
        );
        let dashed = session_in(dir.path(), "/home/u/my-app", "dashed");
        let nested = session_in(dir.path(), "/home/u/my/app", "nested");
        let spaced = session_in(dir.path(), "/home/u/my app", "spaced");
        for (cwd, expected) in [
            ("/home/u/my-app", &dashed),
            ("/home/u/my/app", &nested),
            ("/home/u/my app", &spaced),
        ] {
            let full = discover_sessions(dir.path(), Some(cwd)).unwrap();
            assert_eq!(full.len(), 1, "{cwd}");
            assert_eq!(&full[0].id, expected, "{cwd}");
            let headers = discover_session_headers(dir.path(), Some(cwd)).unwrap();
            assert_eq!(headers.len(), 1, "{cwd}");
            assert_eq!(&headers[0].id, expected, "{cwd}");
            assert_eq!(
                latest_session(dir.path(), Some(cwd)).unwrap().unwrap().id,
                *expected
            );
        }
        // Without a cwd, everything is still listed.
        assert_eq!(discover_sessions(dir.path(), None).unwrap().len(), 3);
    }

    #[test]
    fn colliding_unicode_and_punctuation_cwds_stay_separate() {
        let dir = tempdir().unwrap();
        let a = session_in(dir.path(), "/work/café:x", "a");
        let b = session_in(dir.path(), "/work/café/x", "b");
        let c = session_in(dir.path(), "/work/cafe\u{301}/x", "c");
        let only = |cwd: &str| -> Vec<String> {
            discover_sessions(dir.path(), Some(cwd))
                .unwrap()
                .into_iter()
                .map(|s| s.id)
                .collect()
        };
        assert_eq!(only("/work/café:x"), vec![a]);
        assert_eq!(only("/work/café/x"), vec![b]);
        assert_eq!(only("/work/cafe\u{301}/x"), vec![c]);
    }

    #[test]
    fn existing_directories_stay_discoverable_across_cwd_spellings() {
        let dir = tempdir().unwrap();
        // The same directory recorded with and without a trailing separator.
        let plain = session_in(dir.path(), "/tmp/work", "plain");
        assert!(same_cwd("/tmp/work/", "/tmp/work"));
        let found = discover_sessions(dir.path(), Some("/tmp/work")).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, plain);
        assert!(same_cwd(r"C:\Users\a", "C:/Users/a"));
        assert!(same_cwd("/", "/"));
        assert!(!same_cwd("/a/b-c", "/a/b/c"));
        // A header with no recorded cwd cannot be disproved, so it is kept.
        assert!(same_cwd("", "/anything"));
        assert_eq!(same_cwd(r"C:\Dev\App", "c:/dev/app"), cfg!(windows));
        // A session filed under the pre-alignment directory name is still read.
        let legacy_dir = dir.path().join(legacy_encode_cwd_component("/tmp/old"));
        std::fs::create_dir_all(&legacy_dir).unwrap();
        let header = crate::JsonlV4Header {
            kind: "header".into(),
            version: 4,
            id: "legacy-1".into(),
            created_at: 1,
            cwd: "/tmp/old".into(),
            parent_session_id: None,
            legacy_parent_session_path: None,
            metadata: None,
        };
        std::fs::write(
            legacy_dir.join("legacy.jsonl"),
            crate::codec::encode_header(&header),
        )
        .unwrap();
        let found = discover_sessions(dir.path(), Some("/tmp/old")).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "legacy-1");
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
        let found = discover_sessions(dir.path(), Some("/tmp/work"))
            .unwrap()
            .into_iter()
            .find(|session| session.id == first.header.id)
            .unwrap();
        assert_eq!(found.all_messages_text, "a");
    }

    #[test]
    fn runtime_sidecar_logs_are_not_sessions_and_header_listing_is_lightweight() {
        let dir = tempdir().unwrap();
        let mut session = JsonlSession::create(dir.path(), "/tmp/work", Some("one")).unwrap();
        session
            .append_entry(crate::SessionEntry::message(
                "user",
                serde_json::json!([{"type":"text","text":"searchable"}]),
            ))
            .unwrap();
        let sidecar = session.path.with_extension("runtime.jsonl");
        std::fs::write(&sidecar, "{\"type\":\"runtime\",\"schema_version\":1}\n").unwrap();

        let found = discover_sessions(dir.path(), None).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, session.path);
        assert_eq!(found[0].all_messages_text, "searchable");

        let headers = discover_session_headers(dir.path(), None).unwrap();
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].path, session.path);
        assert!(headers[0].all_messages_text.is_empty());
        assert_eq!(headers[0].message_count, 0);
    }

    #[test]
    fn digest_keeps_head_and_tail_of_a_long_session_within_its_bound() {
        let dir = tempdir().unwrap();
        let mut session = JsonlSession::create(dir.path(), "/tmp/work", Some("long")).unwrap();
        let turns = 400;
        let mut full = Vec::new();
        for turn in 0..turns {
            let text = format!("turn-{turn:04} {}", "word ".repeat(100));
            full.push(text.clone());
            session
                .append_entry(crate::SessionEntry::message(
                    "user",
                    serde_json::json!([{"type": "text", "text": text}]),
                ))
                .unwrap();
        }
        let full = full.join(" ");
        assert!(full.len() > 3 * (DIGEST_HEAD_BYTES + DIGEST_TAIL_BYTES));
        let found = discover_sessions(dir.path(), None).unwrap();
        let summary = &found[0];
        assert_eq!(summary.message_count, turns);
        assert_eq!(summary.messages_text_bytes, full.len());
        let text = &summary.all_messages_text;
        assert!(text.len() <= DIGEST_HEAD_BYTES + DIGEST_TAIL_BYTES + " … ".len());
        assert!(text.starts_with("turn-0000 "), "{}", &text[..40]);
        assert!(text.ends_with(&full[full.len() - 1000..]));
        assert!(text.contains(" … "));
        // A short session is untouched.
        let digest = {
            let mut digest = Digest::default();
            digest.push("a");
            digest.push("b");
            digest.finish()
        };
        assert_eq!(digest, ("a b".to_string(), 3));
    }

    #[test]
    fn digest_bounds_hold_across_multibyte_text() {
        let mut digest = Digest::default();
        for _ in 0..50_000 {
            digest.push("é漢");
        }
        let (text, bytes) = digest.finish();
        assert_eq!(bytes, 50_000 * "é漢".len() + 49_999);
        assert!(text.len() <= DIGEST_HEAD_BYTES + DIGEST_TAIL_BYTES + " … ".len());
    }

    #[test]
    fn discovery_skips_oversized_headers_and_records() {
        let dir = tempdir().unwrap();
        let mut session = JsonlSession::create(dir.path(), "/tmp/work", Some("ok")).unwrap();
        session
            .append_entry(crate::SessionEntry::message(
                "user",
                serde_json::json!([{"type": "text", "text": "kept"}]),
            ))
            .unwrap();
        // A record far past the per-line cap, then one more normal message.
        std::fs::OpenOptions::new()
            .append(true)
            .open(&session.path)
            .unwrap()
            .write_all(&vec![b'z'; MAX_DIGEST_LINE_BYTES + 1])
            .unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&session.path)
            .unwrap()
            .write_all(b"\n")
            .unwrap();
        let sub = session.path.parent().unwrap();
        std::fs::write(
            sub.join("huge-header.jsonl"),
            vec![b'{'; MAX_HEADER_LINE_BYTES * 2],
        )
        .unwrap();
        let found = discover_sessions(dir.path(), None).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].all_messages_text, "kept");
        assert_eq!(discover_session_headers(dir.path(), None).unwrap().len(), 1);
    }

    #[test]
    fn session_dir_resolution_matches_ts_order() {
        let previous = std::env::var_os("PI_CODING_AGENT_SESSION_DIR");
        std::env::remove_var("PI_CODING_AGENT_SESSION_DIR");
        assert_eq!(
            resolve_session_dir_from(Some("~/sessions"), Some("/settings/sessions")),
            expand_tilde("~/sessions")
        );
        assert_eq!(
            resolve_session_dir_from(None, Some("~/from-settings")),
            expand_tilde("~/from-settings")
        );
        std::env::set_var("PI_CODING_AGENT_SESSION_DIR", "/env/sessions");
        assert_eq!(
            resolve_session_dir_from(None, Some("/settings/sessions")),
            PathBuf::from("/env/sessions")
        );
        std::env::remove_var("PI_CODING_AGENT_SESSION_DIR");
        match previous {
            Some(value) => std::env::set_var("PI_CODING_AGENT_SESSION_DIR", value),
            None => std::env::remove_var("PI_CODING_AGENT_SESSION_DIR"),
        }
    }

    #[test]
    fn discover_sessions_finds_all_sessions_across_subdirs_and_root() {
        let dir = tempdir().unwrap();
        // Create session in a subdirectory
        let sub = JsonlSession::create(dir.path(), "/tmp/work1", Some("sub-session")).unwrap();
        // Create session in another subdirectory
        let sub2 = JsonlSession::create(dir.path(), "/tmp/work2", Some("sub-session-2")).unwrap();
        // Create session directly in session root
        let root_file = dir.path().join("root-session.jsonl");
        let header = crate::JsonlV4Header {
            kind: "header".into(),
            version: 4,
            id: "root-123".into(),
            created_at: 1234567890u64,
            cwd: "/tmp/root".into(),
            parent_session_id: None,
            legacy_parent_session_path: None,
            metadata: Some(serde_json::json!({ "name": "root-session" })),
        };
        std::fs::write(&root_file, crate::codec::encode_header(&header)).unwrap();

        let all = discover_sessions(dir.path(), None).unwrap();
        assert_eq!(all.len(), 3);
        let ids: Vec<String> = all.iter().map(|s| s.id.clone()).collect();
        assert!(ids.contains(&sub.header.id));
        assert!(ids.contains(&sub2.header.id));
        assert!(ids.contains(&"root-123".to_string()));
    }
}
