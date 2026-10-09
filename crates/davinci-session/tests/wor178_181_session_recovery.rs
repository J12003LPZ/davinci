use std::io::Write;

use davinci_session::{
    JsonlCreateOptions, JsonlSession, JsonlSessionRepo, JsonlStoredSession, SessionEntry,
};
use serde_json::json;

/// The first two bytes of a four-byte UTF-8 code point: what a write torn
/// mid-character leaves behind.
const TORN_CODE_POINT: [u8; 2] = [0xF0, 0x9F];

fn append_bytes(path: &std::path::Path, bytes: &[u8]) {
    let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    file.write_all(bytes).unwrap();
}

#[test]
fn legacy_session_recovers_a_partial_utf8_tail() {
    let directory = tempfile::tempdir().unwrap();
    let mut session = JsonlSession::create(directory.path(), "fixture", None).unwrap();
    session
        .append_entry(SessionEntry::message("user", json!("kept")))
        .unwrap();
    let path = session.path.clone();
    drop(session);
    let intact = std::fs::read(&path).unwrap();

    let mut torn = br#"{"kind":"entry","entry":{"id":"x","text":""#.to_vec();
    torn.extend_from_slice(&TORN_CODE_POINT);
    append_bytes(&path, &torn);

    let mut reopened = JsonlSession::open(&path).unwrap();
    assert_eq!(reopened.entries.len(), 1);
    // The next write repairs the tail and keeps the torn bytes as evidence.
    reopened
        .append_entry(SessionEntry::message("user", json!("after")))
        .unwrap();
    let repaired = std::fs::read(&path).unwrap();
    assert!(repaired.starts_with(&intact));
    assert!(std::str::from_utf8(&repaired).is_ok());
    let backups: Vec<_> = std::fs::read_dir(directory.path().join(path.parent().unwrap()))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().contains(".torn-"))
        .collect();
    assert_eq!(backups.len(), 1);
    assert_eq!(std::fs::read(backups[0].path()).unwrap(), torn);
    assert_eq!(JsonlSession::open(&path).unwrap().entries.len(), 2);
}

#[test]
fn stored_session_recovers_a_partial_utf8_tail() {
    let directory = tempfile::tempdir().unwrap();
    let repo = JsonlSessionRepo::new(directory.path());
    let mut stored = repo
        .create(JsonlCreateOptions {
            id: None,
            cwd: "fixture".into(),
            parent_session_id: None,
            metadata: None,
        })
        .unwrap();
    let kept = stored
        .append_entry(SessionEntry::message("user", json!("kept")), "main")
        .unwrap();
    let path = stored.info().path.clone();
    drop(stored);
    let intact = std::fs::read(&path).unwrap();

    let mut torn = br#"{"kind":"entry","entry":{"id":"x","text":""#.to_vec();
    torn.extend_from_slice(&TORN_CODE_POINT);
    append_bytes(&path, &torn);

    let loaded = JsonlStoredSession::load(&path).unwrap();
    assert_eq!(loaded.get_leaf_id().as_deref(), Some(kept.id.as_str()));
    assert!(loaded.get_entry("x").is_none());
    assert_eq!(std::fs::read(&path).unwrap(), intact);
}

#[test]
fn invalid_utf8_in_a_completed_record_is_still_an_error() {
    let directory = tempfile::tempdir().unwrap();
    let mut session = JsonlSession::create(directory.path(), "fixture", None).unwrap();
    session
        .append_entry(SessionEntry::message("user", json!("kept")))
        .unwrap();
    let path = session.path.clone();
    drop(session);
    append_bytes(&path, b"{\"bad\":\"\xF0\x9F\"}\n");
    assert!(JsonlSession::open(&path).is_err());
    assert!(JsonlStoredSession::load(&path).is_err());
}

fn write_first_line(first: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("s.jsonl");
    std::fs::write(&path, format!("{first}\n")).unwrap();
    (directory, path)
}

#[test]
fn unsupported_version_header_with_nested_role_is_not_migrated() {
    for metadata in [json!({"role": "worker"}), json!({"type": "x"})] {
        let header = json!({
            "kind": "header", "version": 5, "id": "s", "createdAt": 1,
            "cwd": "/w", "metadata": metadata,
        });
        let (_directory, path) = write_first_line(&header.to_string());
        let Err(err) = JsonlSession::open(&path) else {
            panic!("must fail closed");
        };
        assert!(
            err.to_string().contains("unsupported session version"),
            "{err}"
        );
    }
}

#[test]
fn genuine_v3_header_still_migrates() {
    let (_directory, path) = write_first_line(
        r#"{"type":"session","version":3,"id":"abc","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/w"}"#,
    );
    let session = JsonlSession::open(&path).unwrap();
    assert_eq!(session.header.version, 4);
    assert_eq!(session.header.id, "abc");
}
