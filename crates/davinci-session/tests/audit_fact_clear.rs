use davinci_session::{
    encode_mutation, parse_mutation, user_message_entry, JsonlCreateOptions, JsonlSessionRepo,
    SessionMutation,
};

#[test]
fn audit_cleared_name_roundtrips_through_the_native_codec() {
    let mutation = SessionMutation::FactName { seq: 1, name: None };
    let wire = encode_mutation(&mutation);
    assert_eq!(
        parse_mutation(wire.trim_end()).expect("encoded clear-name must decode"),
        mutation
    );
}

#[test]
fn audit_cleared_label_roundtrips_through_the_native_codec() {
    let mutation = SessionMutation::FactLabel {
        seq: 1,
        target_id: "entry".into(),
        label: None,
    };
    let wire = encode_mutation(&mutation);
    assert_eq!(
        parse_mutation(wire.trim_end()).expect("encoded clear-label must decode"),
        mutation
    );
}

#[test]
fn audit_jsonl_repo_reopens_after_clearing_a_name() {
    let root = tempfile::tempdir().unwrap();
    let repo = JsonlSessionRepo::new(root.path());
    let mut session = repo
        .create(JsonlCreateOptions {
            id: Some("clear-name".into()),
            cwd: root.path().to_string_lossy().into_owned(),
            parent_session_id: None,
            metadata: None,
        })
        .unwrap();
    session.set_name(Some("original")).unwrap();
    session.set_name(None).unwrap();
    let info = session.info().clone();
    drop(session);
    let reopened = repo
        .open(&info)
        .expect("cleared name must remain reloadable");
    assert_eq!(reopened.get_name(), None);
}

#[test]
fn audit_jsonl_repo_reopens_after_clearing_a_label() {
    let root = tempfile::tempdir().unwrap();
    let repo = JsonlSessionRepo::new(root.path());
    let mut session = repo
        .create(JsonlCreateOptions {
            id: Some("clear-label".into()),
            cwd: root.path().to_string_lossy().into_owned(),
            parent_session_id: None,
            metadata: None,
        })
        .unwrap();
    session
        .append_entry(user_message_entry("entry", "fixture"), "main")
        .unwrap();
    session.set_label("entry", Some("original")).unwrap();
    session.set_label("entry", None).unwrap();
    let info = session.info().clone();
    drop(session);
    let reopened = repo
        .open(&info)
        .expect("cleared label must remain reloadable");
    assert_eq!(reopened.get_label("entry"), None);
}
