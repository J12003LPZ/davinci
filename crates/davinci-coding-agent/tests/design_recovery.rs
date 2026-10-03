use davinci_agent::{Agent, PermissionMode};
use davinci_coding_agent::design::{admission::*, records::*, store::*, types::*};
use davinci_session::JsonlSession;
use std::collections::BTreeMap;

#[test]
fn torn_session_tail_is_preserved_before_recovery_and_new_publication() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let mut session = JsonlSession::create_in_directory(dir.path(), "fixture", None).unwrap();
    let path = session.path.clone();
    session
        .append_entry(davinci_session::custom_entry(
            "before",
            "fixture",
            serde_json::json!({}),
        ))
        .unwrap();
    drop(session);
    let torn = b"{\"type\":\"entry\",\"entry\":{\"uncommitted\":true";
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(torn)
        .unwrap();
    let mut reopened = JsonlSession::open(&path).unwrap();
    reopened
        .append_entry(davinci_session::custom_entry(
            "after",
            "fixture",
            serde_json::json!({}),
        ))
        .unwrap();
    let backups: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "bak"))
        .collect();
    assert_eq!(
        backups.len(),
        1,
        "recovery must retain corrupt evidence before truncating it"
    );
    assert_eq!(std::fs::read(&backups[0]).unwrap(), torn);
    assert_eq!(JsonlSession::open(&path).unwrap().entries.len(), 2);
}

#[test]
fn terminated_corrupt_events_are_not_replaced_and_concurrent_writers_are_excluded() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let mut first = JsonlSession::create_in_directory(dir.path(), "fixture", None).unwrap();
    let path = first.path.clone();
    first
        .append_entry(davinci_session::custom_entry(
            "first",
            "fixture",
            serde_json::json!({}),
        ))
        .unwrap();
    let mut stale = JsonlSession::open(&path).unwrap();
    let entry = davinci_session::custom_entry("second", "fixture", serde_json::json!({}));
    assert!(
        stale.append_entry(entry.clone()).is_err(),
        "a live writer owns publication"
    );
    drop(first);
    // A failed writer must be reopened; its cached branch cannot overwrite the WAL.
    let mut second = JsonlSession::open(&path).unwrap();
    second.append_entry(entry).unwrap();
    assert_eq!(second.entries.len(), 2);
    drop(second);
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{corrupt}\n")
        .unwrap();
    let original = std::fs::read(&path).unwrap();
    assert!(JsonlSession::open(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
}

#[test]
fn cleanup_preserves_committed_branches_and_restart_source_without_node() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut agent = Agent::new("offline fixture");
    agent.set_permission_mode(PermissionMode::AlwaysApprove);
    agent.session = Some(
        JsonlSession::create_in_directory(&root.join("sessions"), &root.to_string_lossy(), None)
            .unwrap(),
    );
    let ctx = AuthorizedDesignContext::from_agent(&agent, &root).unwrap();
    let store = DesignStore::new(root.join("design"));
    let s = agent.session.as_mut().unwrap();
    let created = store
        .create(
            &ctx,
            s,
            CreateDesign {
                title: "Recovery".into(),
                brief: "Retain both branches".into(),
                kind: DesignKind::Landing,
                variants: 1,
                operation_id: OperationId::new(),
            },
        )
        .unwrap();
    let created_leaf = s.leaf_id.clone();
    let sources = store
        .store_sources(
            &ctx,
            s,
            &BTreeMap::from([("index.html".into(), "<h1>Preserved</h1>".into())]),
            vec!["index.html".into()],
        )
        .unwrap();
    let revision = store
        .commit_revision(
            &ctx,
            s,
            RevisionWrite {
                artifact_id: created.id,
                expected_revision: RevisionId(0),
                operation_id: OperationId::new(),
                sources,
                variants: vec![Variant {
                    id: VariantId::new(),
                    title: "One".into(),
                    artboards: vec![Artboard {
                        id: ArtboardId::new(),
                        title: "Home".into(),
                        entry_point: "index.html".into(),
                    }],
                }],
                bindings: vec![],
                assets: vec![],
                profile_refs: vec![],
                system_snapshot: None,
            },
        )
        .unwrap();
    let committed_leaf = s.leaf_id.clone();
    // Simulate interruption between blob publication and the session commit.
    store
        .store_sources(
            &ctx,
            s,
            &BTreeMap::from([("orphan.html".into(), "uncommitted".into())]),
            vec!["orphan.html".into()],
        )
        .unwrap();
    s.leaf_id = created_leaf;
    assert_eq!(store.prune_unreferenced(&ctx, s).unwrap(), 11);
    s.leaf_id = committed_leaf;
    assert_eq!(
        store
            .read_revision(&ctx, s, created.id, RevisionId(1))
            .unwrap(),
        revision
    );
    assert_eq!(store.prune_unreferenced(&ctx, s).unwrap(), 0);
    let path = s.path.clone();
    agent.session = None;
    let mut reopened = JsonlSession::open(&path).unwrap();
    assert_eq!(
        store
            .read_sources(&ctx, &reopened, created.id, RevisionId(1))
            .unwrap()["index.html"],
        "<h1>Preserved</h1>"
    );
    let source = store
        .blob_directory(&ctx)
        .join(&revision.sources.files["index.html"].relative_store_path);
    std::fs::remove_file(source).unwrap();
    assert!(store.prune_unreferenced(&ctx, &mut reopened).is_err());
    assert!(store
        .read_revision(&ctx, &reopened, created.id, RevisionId(1))
        .is_err());
    assert!(
        path.is_file(),
        "corrupt evidence must retain the session for diagnosis"
    );
}
