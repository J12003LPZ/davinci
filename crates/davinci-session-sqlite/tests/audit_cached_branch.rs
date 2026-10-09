use davinci_session::user_message_entry;
use davinci_session_sqlite::SqliteSessionStore;

#[test]
fn audit_cached_branch_stops_at_the_requested_leaf() {
    let root = tempfile::tempdir().unwrap();
    let store = SqliteSessionStore::open(&root.path().join("sessions.db")).unwrap();
    let mut session = store.create_repo_session("branch").unwrap();
    session
        .append_entry(user_message_entry("root", "root request"), "main")
        .unwrap();
    session
        .append_entry(user_message_entry("later", "later request"), "main")
        .unwrap();
    store.persist_session(&session).unwrap();

    assert_eq!(
        store.cached_branch_ids("branch", "root").unwrap(),
        vec!["root".to_string()],
        "cached path includes descendants after the requested leaf"
    );
}

#[test]
fn cached_branch_matches_the_path_for_every_leaf_of_a_chain() {
    let root = tempfile::tempdir().unwrap();
    let store = SqliteSessionStore::open(&root.path().join("sessions.db")).unwrap();
    let mut session = store.create_repo_session("chain").unwrap();
    for id in ["a", "b", "c", "d"] {
        session
            .append_entry(user_message_entry(id, id), "main")
            .unwrap();
    }
    store.persist_session(&session).unwrap();

    let expected = ["a", "b", "c", "d"];
    for (index, leaf) in expected.iter().enumerate() {
        let want: Vec<String> = expected[..=index].iter().map(|id| id.to_string()).collect();
        assert_eq!(
            store.cached_branch_ids("chain", leaf).unwrap(),
            want,
            "leaf {leaf}"
        );
    }
}
