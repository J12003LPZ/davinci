use pi_session::manager::SessionManager;
use pi_session::types::SessionEntry;
use pi_session_sqlite::SqliteSessionBackend;
use tempfile::tempdir;

#[test]
fn test_sqlite_session_backend_fts() {
    let backend = SqliteSessionBackend::open_in_memory().expect("open sqlite in memory");
    let header = pi_session::types::SessionHeader {
        entry_type: "session".to_string(),
        version: 4,
        id: "sess-1".to_string(),
        timestamp: "2026-08-29T00:00:00Z".to_string(),
        cwd: "/workspace".to_string(),
        parent_session: None,
    };
    backend.insert_session(&header).expect("insert session");

    let entry = SessionEntry::Compaction {
        id: "e-1".to_string(),
        parent_id: None,
        timestamp: "2026-08-29T00:00:01Z".to_string(),
        summary: "Refactored the authentication database layer".to_string(),
        first_kept_entry_id: "e-0".to_string(),
        tokens_before: 5000,
    };
    backend
        .insert_entry("sess-1", &entry)
        .expect("insert entry");

    let matches = backend
        .search_fts("sess-1", "authentication")
        .expect("fts search");
    assert_eq!(matches, vec!["e-1"]);
}

#[test]
fn test_session_manager_list_and_read() {
    let tmp = tempdir().expect("tempdir");
    let mgr = SessionManager::new(tmp.path());
    let (id, _) = mgr
        .create_session("/workspace", None)
        .expect("create session");

    let entry = SessionEntry::ModelChange {
        id: "m-1".to_string(),
        parent_id: None,
        timestamp: "2026-08-29T00:00:00Z".to_string(),
        provider: "anthropic".to_string(),
        model_id: "claude-sonnet-4-5".to_string(),
    };
    mgr.append_entry(&id, &entry).expect("append entry");

    let sessions = mgr.list_sessions().expect("list sessions");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, id);

    let entries = mgr.read_entries(&id).expect("read entries");
    assert_eq!(entries.len(), 1);
}
