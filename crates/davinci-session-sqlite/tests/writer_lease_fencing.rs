//! WOR-172: a released lease keeps its fence, so a stale handle from an
//! earlier acquisition never matches a later one, even under the same owner.
use davinci_session_sqlite::SqliteSessionStore;

fn store(root: &tempfile::TempDir) -> SqliteSessionStore {
    SqliteSessionStore::open(&root.path().join("sessions.db")).unwrap()
}

#[test]
fn reacquired_lease_rejects_the_previous_owners_stale_fence() {
    let root = tempfile::tempdir().unwrap();
    let store = store(&root);
    store.create_repo_session("session").unwrap();
    let original = store
        .acquire_writer_lease("session", "same-owner", 1000, 2000)
        .unwrap()
        .unwrap();
    store.release_writer_lease("session", &original).unwrap();
    let reacquired = store
        .acquire_writer_lease("session", "same-owner", 1001, 3000)
        .unwrap()
        .unwrap();

    let mut stale = original.clone();
    let stale_renewal = store
        .renew_writer_lease("session", &mut stale, 1002, 9999)
        .unwrap();
    assert!(
        reacquired.fence > original.fence && !stale_renewal,
        "old fence={} new fence={} stale renewal accepted={stale_renewal}",
        original.fence,
        reacquired.fence
    );
    let mut live = reacquired.clone();
    assert!(store
        .renew_writer_lease("session", &mut live, 1002, 4000)
        .unwrap());
}

#[test]
fn stale_release_does_not_delete_reacquired_lease() {
    let root = tempfile::tempdir().unwrap();
    let store = store(&root);
    store.create_repo_session("session").unwrap();
    let original = store
        .acquire_writer_lease("session", "same-owner", 1000, 2000)
        .unwrap()
        .unwrap();
    store.release_writer_lease("session", &original).unwrap();
    let _reacquired = store
        .acquire_writer_lease("session", "same-owner", 1001, 3000)
        .unwrap()
        .unwrap();
    store.release_writer_lease("session", &original).unwrap();
    let other = store
        .acquire_writer_lease("session", "other-owner", 1002, 4000)
        .unwrap();
    assert!(
        other.is_none(),
        "stale release allowed another owner to acquire"
    );
}

#[test]
fn released_lease_is_free_immediately_and_cannot_be_renewed() {
    let root = tempfile::tempdir().unwrap();
    let store = store(&root);
    store.create_repo_session("session").unwrap();
    let lease = store
        .acquire_writer_lease("session", "a", 1000, 50_000)
        .unwrap()
        .unwrap();
    store.release_writer_lease("session", &lease).unwrap();
    let mut released = lease.clone();
    assert!(!store
        .renew_writer_lease("session", &mut released, 1001, 60_000)
        .unwrap());
    let next = store
        .acquire_writer_lease("session", "b", 1001, 60_000)
        .unwrap()
        .expect("release frees the lease before its old expiry");
    assert_eq!(next.fence, lease.fence + 1);
}

#[test]
fn fence_generation_survives_reopen_after_release() {
    let root = tempfile::tempdir().unwrap();
    let original = {
        let store = store(&root);
        store.create_repo_session("session").unwrap();
        let lease = store
            .acquire_writer_lease("session", "same-owner", 1000, 2000)
            .unwrap()
            .unwrap();
        store.release_writer_lease("session", &lease).unwrap();
        lease
    };
    let store = store(&root);
    let reacquired = store
        .acquire_writer_lease("session", "same-owner", 1001, 3000)
        .unwrap()
        .unwrap();
    assert_eq!(reacquired.fence, original.fence + 1);
    store.release_writer_lease("session", &original).unwrap();
    assert!(store
        .acquire_writer_lease("session", "other-owner", 1002, 4000)
        .unwrap()
        .is_none());
}
