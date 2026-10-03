use davinci_session::{EntryQuery, SessionEntry};
use davinci_session_sqlite::SqliteSessionStore;
use serde_json::{json, Value};

fn result_entry(result: Value) -> SessionEntry {
    let mut entry = SessionEntry::message("toolResult", json!({"content": result}));
    entry.message = Some(result);
    entry
}

#[test]
fn duplicate_operation_event_after_reopen_is_one_logical_entry() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sessions.sqlite");
    let store = SqliteSessionStore::open(&path).unwrap();
    store.create_repo_session("session-1").unwrap();
    let event_id = "operation-result:session-1:call-1";
    let entry = result_entry(
        json!({"role":"toolResult","toolCallId":"call-1","content":[{"type":"text","text":"ok"}]}),
    );
    store
        .append_entry_once("session-1", event_id, None, entry.clone())
        .unwrap();
    drop(store);

    let reopened = SqliteSessionStore::open(&path).unwrap();
    reopened
        .append_entry_once("session-1", event_id, None, entry)
        .unwrap();
    let session = reopened.open_repo_session("session-1").unwrap();
    let entries = session.find_entries(&EntryQuery::default()).unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].extra["operationEventId"], event_id);
}

#[test]
fn sqlite_projection_rejects_stale_lineage_and_event_id_collisions() {
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteSessionStore::open(&directory.path().join("sessions.sqlite")).unwrap();
    store.create_repo_session("session-1").unwrap();
    store
        .append_entry_once(
            "session-1",
            "operation-result:session-1:root",
            None,
            result_entry(json!({"text":"root"})),
        )
        .unwrap();

    let stale = store
        .append_entry_once(
            "session-1",
            "operation-result:session-1:child",
            None,
            result_entry(json!({"text":"child"})),
        )
        .unwrap_err();
    assert!(stale.message.contains("lineage"));

    let collision = store
        .append_entry_once(
            "session-1",
            "operation-result:session-1:root",
            Some("operation-result:session-1:root"),
            result_entry(json!({"text":"corrupt"})),
        )
        .unwrap_err();
    assert!(collision.message.contains("different payload"));
}
