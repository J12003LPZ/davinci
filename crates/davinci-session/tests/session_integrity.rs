//! Regressions for JSONL session ownership, fork lineage, and replay
//! integrity (WOR-175, 176, 177, 180, 182, 183).

use davinci_session::{
    build_context_entries, encode_header, encode_mutation, operation_started, JsonlCreateOptions,
    JsonlSession, JsonlSessionRepo, JsonlStoredSession, JsonlV4Header, SessionEntry,
    SessionMutation,
};
use serde_json::json;
use std::io::Write;
use std::path::Path;

fn text_of(entry: &SessionEntry) -> String {
    entry.message.as_ref().unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string()
}

fn user(text: &str) -> SessionEntry {
    SessionEntry::message("user", json!(text))
}

fn create(repo: &JsonlSessionRepo) -> JsonlStoredSession {
    repo.create(JsonlCreateOptions {
        id: Some("fixture".into()),
        cwd: "/fixture".into(),
        parent_session_id: None,
        metadata: None,
    })
    .unwrap()
}

fn header_only(path: &Path) {
    let header = JsonlV4Header {
        kind: "header".into(),
        version: 4,
        id: "fixture".into(),
        created_at: 1,
        cwd: "/fixture".into(),
        parent_session_id: None,
        legacy_parent_session_path: None,
        metadata: None,
    };
    std::fs::write(path, encode_header(&header)).unwrap();
}

fn append(path: &Path, mutation: &SessionMutation) {
    let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    file.write_all(encode_mutation(mutation).as_bytes())
        .unwrap();
}

#[test]
fn stale_stored_handle_cannot_corrupt_shared_sequence() {
    let dir = tempfile::tempdir().unwrap();
    let repo = JsonlSessionRepo::new(dir.path());
    let mut first = create(&repo);
    let mut stale = repo.open(&first.info).unwrap();

    first.append_message("first").unwrap();
    assert!(stale.append_message("stale").is_err());

    let mut reopened = repo.open(&first.info).unwrap();
    assert_eq!(reopened.get_stats().message_count, 1);
    reopened.append_message("after reopen").unwrap();
    assert_eq!(repo.open(&first.info).unwrap().get_stats().message_count, 2);
}

#[test]
fn fork_omits_abandoned_branch_messages() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = JsonlSession::create(dir.path(), "/fixture", None).unwrap();
    session.append_entry(user("root")).unwrap();
    let root = session.leaf_id.clone();
    session
        .append_entry(user("abandoned private marker"))
        .unwrap();
    session.set_leaf(root);
    session.append_entry(user("active request")).unwrap();
    let leaf = session.leaf_id.clone().unwrap();

    let forked = session.fork(&leaf, dir.path()).unwrap();
    let texts: Vec<_> = forked.entries.iter().map(text_of).collect();
    assert_eq!(texts, ["root", "active request"]);
}

#[test]
fn fork_retains_compaction_first_kept_reference() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = JsonlSession::create(dir.path(), "/fixture", None).unwrap();
    session.append_entry(user("old request")).unwrap();
    session.append_entry(user("retained request")).unwrap();
    let retained = session.leaf_id.clone().unwrap();
    let mut compaction = SessionEntry::message("user", json!(""));
    compaction.entry_type = "compaction".into();
    compaction.message = None;
    compaction.extra.insert("summary".into(), json!("summary"));
    compaction
        .extra
        .insert("firstKeptEntryId".into(), json!(retained));
    session.append_entry(compaction).unwrap();
    let leaf = session.leaf_id.clone().unwrap();

    let forked = JsonlSession::open(&session.fork(&leaf, dir.path()).unwrap().path).unwrap();
    let context = build_context_entries(&forked.entries, forked.leaf_id.as_deref());
    let texts: Vec<_> = context
        .iter()
        .filter(|entry| entry.message.is_some())
        .map(|entry| text_of(entry))
        .collect();
    assert_eq!(texts, ["retained request"]);
}

#[test]
fn clone_omits_abandoned_branch_and_preserves_selected_leaf() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = JsonlSession::create(dir.path(), "/fixture", None).unwrap();
    session.append_entry(user("root")).unwrap();
    let root = session.leaf_id.clone();
    session.append_entry(user("abandoned")).unwrap();
    session.set_leaf(root);
    session.append_entry(user("selected")).unwrap();
    let selected = session.leaf_id.clone();
    session.append_entry(user("later branch")).unwrap();
    session.set_leaf(selected.clone());

    let cloned = session.clone_session(dir.path()).unwrap();
    let reopened = JsonlSession::open(&cloned.path).unwrap();
    assert_ne!(reopened.header.id, session.header.id);
    assert_eq!(
        reopened.header.parent_session_id,
        Some(session.header.id.clone())
    );
    assert_eq!(reopened.leaf_id, selected);
    assert_eq!(
        reopened.entries.iter().map(text_of).collect::<Vec<_>>(),
        ["root", "selected"]
    );
}

#[test]
fn clone_retains_compaction_and_label_references() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = JsonlSession::create(dir.path(), "/fixture", None).unwrap();
    session.append_entry(user("old")).unwrap();
    session.append_entry(user("retained requirement")).unwrap();
    let retained = session.leaf_id.clone().unwrap();
    let mut compaction = user("");
    compaction.entry_type = "compaction".into();
    compaction.message = None;
    compaction.extra.insert("summary".into(), json!("summary"));
    compaction
        .extra
        .insert("firstKeptEntryId".into(), json!(retained));
    session.append_entry(compaction).unwrap();
    session
        .append_entry(SessionEntry::label_change(&retained, Some("keep")))
        .unwrap();
    session.append_entry(user("latest")).unwrap();

    let cloned = session.clone_session(dir.path()).unwrap();
    let reopened = JsonlSession::open(&cloned.path).unwrap();
    let context = build_context_entries(&reopened.entries, reopened.leaf_id.as_deref());
    assert_eq!(
        context
            .iter()
            .filter(|entry| entry.message.is_some())
            .map(|entry| text_of(entry))
            .collect::<Vec<_>>(),
        ["retained requirement", "latest"]
    );
    assert_eq!(
        davinci_session::resolved_labels(&reopened.entries)
            .get(&retained)
            .and_then(|label| label.0.as_deref()),
        Some("keep")
    );
}

#[test]
fn completed_malformed_final_record_is_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corrupt.jsonl");
    header_only(&path);
    for suffix in [
        "{\"kind\":\"entry\"\n",
        "{\"kind\":\"entry\"\r\n",
        "{\"kind\":\"entry\"\npartial",
    ] {
        header_only(&path);
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(suffix.as_bytes())
            .unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(JsonlStoredSession::load(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
}

#[test]
fn replay_rejects_nonconsecutive_lane_and_name_sequences() {
    let dir = tempfile::tempdir().unwrap();
    for mutation in [
        SessionMutation::Lane {
            seq: 99,
            lane: "side".into(),
            leaf_id: None,
        },
        SessionMutation::FactName {
            seq: 99,
            name: Some("name".into()),
        },
    ] {
        let path = dir.path().join("gap.jsonl");
        header_only(&path);
        append(&path, &mutation);
        let error = JsonlStoredSession::load(&path).unwrap_err();
        assert!(
            error.to_string().contains("non-consecutive seq 99"),
            "{error}"
        );
    }
}

#[test]
fn legacy_reader_preserves_lane_sequence_before_append() {
    let dir = tempfile::tempdir().unwrap();
    let repo = JsonlSessionRepo::new(dir.path());
    let mut stored = create(&repo);
    let first = stored.append_message("first").unwrap();
    stored.create_lane("side", Some(&first)).unwrap();
    let path = stored.info.path.clone();
    drop(stored);

    let mut legacy = JsonlSession::open(&path).unwrap();
    legacy.append_entry(user("second")).unwrap();
    assert_eq!(legacy.entries.last().unwrap().seq, 3);
    drop(legacy);

    let reopened = JsonlStoredSession::load(&path).unwrap();
    assert_eq!(reopened.get_stats().message_count, 2);
}

#[test]
fn replay_exposes_multiple_open_operations_for_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("open.jsonl");
    header_only(&path);
    for (seq, id) in [(1, "run-1"), (2, "run-2")] {
        let mut record = operation_started(id, "main", "run");
        record.seq = seq;
        record.timestamp = 1;
        append(
            &path,
            &SessionMutation::Record {
                lane: Some("main".into()),
                record,
            },
        );
    }

    let mut loaded = JsonlStoredSession::load(&path).unwrap();
    assert_eq!(
        loaded.find_open_operations("main", Some(2)).unwrap().len(),
        2
    );
    // Append planning still refuses a third live operation.
    assert!(loaded
        .append_record(operation_started("run-3", "main", "run"))
        .is_err());
}
