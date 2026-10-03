use davinci_session::{JsonlSession, SessionEntry};
use serde_json::{json, Value};

fn result_entry(result: Value) -> SessionEntry {
    let mut entry = SessionEntry::message("toolResult", json!({"content": result}));
    entry.message = Some(result);
    entry
}

#[test]
fn duplicate_operation_event_after_reopen_is_one_logical_entry() {
    let directory = tempfile::tempdir().unwrap();
    let mut session = JsonlSession::create(directory.path(), "fixture", None).unwrap();
    let event_id = "operation-result:session:call-1";
    let entry = result_entry(
        json!({"role":"toolResult","toolCallId":"call-1","content":[{"type":"text","text":"ok"}]}),
    );
    let path = session.path.clone();

    session
        .append_entry_once(event_id, None, entry.clone())
        .unwrap();
    drop(session);

    let mut reopened = JsonlSession::open(&path).unwrap();
    reopened.append_entry_once(event_id, None, entry).unwrap();

    assert_eq!(reopened.entries.len(), 1);
    assert_eq!(reopened.entries[0].extra["operationEventId"], event_id);
}

#[test]
fn operation_event_with_different_payload_is_blocked() {
    let directory = tempfile::tempdir().unwrap();
    let mut session = JsonlSession::create(directory.path(), "fixture", None).unwrap();
    let event_id = "operation-result:session:call-1";

    session
        .append_entry_once(event_id, None, result_entry(json!({"text":"first"})))
        .unwrap();
    let error = session
        .append_entry_once(event_id, None, result_entry(json!({"text":"corrupt"})))
        .unwrap_err();

    assert_eq!(error.code, "invalid_entry");
    assert!(error.message.contains("different payload"));
    assert_eq!(session.entries.len(), 1);
}

#[test]
fn operation_event_requires_the_current_parent_lineage() {
    let directory = tempfile::tempdir().unwrap();
    let mut session = JsonlSession::create(directory.path(), "fixture", None).unwrap();
    session
        .append_entry(SessionEntry::message("user", json!("root")))
        .unwrap();

    let error = session
        .append_entry_once(
            "operation-result:session:call-1",
            None,
            result_entry(json!({"text":"result"})),
        )
        .unwrap_err();

    assert_eq!(error.code, "invalid_entry");
    assert!(error.message.contains("lineage"));
}
