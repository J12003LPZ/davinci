//! SQLite session backend with official fenced writer-lease semantics.

pub mod leases;
pub mod repo;
pub mod schema;

pub use leases::{
    acquire_writer_lease, delete_writer_lease, read_writer_lease, release_writer_lease,
    renew_writer_lease,
};
pub use repo::{SessionHandle, SqliteSessionRepository, StoreError};

#[cfg(test)]
mod tests {
    use super::*;
    use pi_core::{Role, SessionErrorCode, WriterLease, WriterLeaseOptions};

    fn repo() -> SqliteSessionRepository {
        SqliteSessionRepository::open_in_memory(WriterLeaseOptions::default()).unwrap()
    }

    #[test]
    fn rejects_invalid_lease_timing() {
        let bad = WriterLeaseOptions {
            ttl_ms: 0,
            heartbeat_interval_ms: 10,
        };
        assert!(SqliteSessionRepository::open_in_memory(bad).is_err());
        let bad = WriterLeaseOptions {
            ttl_ms: 10,
            heartbeat_interval_ms: 10,
        };
        assert!(SqliteSessionRepository::open_in_memory(bad).is_err());
    }

    #[test]
    fn create_list_open_without_second_writer() {
        let repo = repo();
        let created = repo
            .create(
                Some("session-1"),
                "/tmp/work",
                None,
                Some(&serde_json::json!({"profile":"reviewer"})),
            )
            .unwrap();
        assert_eq!(created.metadata.id, "session-1");
        assert_eq!(
            created.metadata.metadata,
            Some(serde_json::json!({"profile":"reviewer"}))
        );

        let listed = repo.list(Some("/tmp/work")).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(
            listed[0].metadata,
            Some(serde_json::json!({"profile":"reviewer"}))
        );

        // Same-process reopen reuses the storage (no second lease).
        let reopened = repo.open("session-1").unwrap();
        assert_eq!(reopened.metadata.id, "session-1");
    }

    #[test]
    fn lists_without_acquiring_leases() {
        let a = repo();
        a.create(Some("s1"), "/tmp", None, None).unwrap();
        let listed = a.list(None).unwrap();
        assert_eq!(listed.len(), 1);
        // A second repository on a distinct in-memory db cannot see it;
        // list on the same repo must not take the lease away.
        let lease = a.current_lease("s1").unwrap().unwrap();
        assert_eq!(lease.fence, 1);
    }

    #[test]
    fn rejects_second_writer_until_release() {
        let path = tempfile::NamedTempFile::new().unwrap();
        let path = path.path().to_path_buf();
        let first =
            SqliteSessionRepository::open_path(&path, WriterLeaseOptions::default()).unwrap();
        first.create(Some("shared"), "/tmp", None, None).unwrap();
        let second =
            SqliteSessionRepository::open_path(&path, WriterLeaseOptions::default()).unwrap();
        let err = second.open("shared").unwrap_err();
        match err {
            StoreError::Session(e) => {
                assert_eq!(e.code, SessionErrorCode::Storage);
                assert!(e.message.contains("already has an active writer"));
            }
            other => panic!("unexpected {other:?}"),
        }
        first.release("shared").unwrap();
        assert!(second.open("shared").is_ok());
    }

    #[test]
    fn fences_stale_owner_after_expiry() {
        let path = tempfile::NamedTempFile::new().unwrap();
        let path = path.path().to_path_buf();
        let first =
            SqliteSessionRepository::open_path(&path, WriterLeaseOptions::default()).unwrap();
        first.create(Some("lease"), "/tmp", None, None).unwrap();
        first.force_expire_lease("lease", 1).unwrap();

        let second =
            SqliteSessionRepository::open_path(&path, WriterLeaseOptions::default()).unwrap();
        second.open("lease").unwrap();
        let lease = second.current_lease("lease").unwrap().unwrap();
        assert_eq!(lease.fence, 2);

        let err = first
            .append_message("lease", "main", "e1", Role::User, "stale")
            .unwrap_err();
        match err {
            StoreError::Session(e) => {
                assert!(e.message.contains("writer lease was lost"));
            }
            other => panic!("unexpected {other:?}"),
        }

        first.release("lease").unwrap();
        let still = second.current_lease("lease").unwrap().unwrap();
        assert_eq!(still.fence, 2);
        assert_eq!(still.owner_id, lease.owner_id);
    }

    #[test]
    fn assigns_parents_and_one_sequence() {
        let repo = repo();
        repo.create(Some("s"), "/tmp", None, None).unwrap();
        let a = repo
            .append_message("s", "main", "m1", Role::User, "one")
            .unwrap();
        let b = repo
            .append_message("s", "main", "m2", Role::Assistant, "two")
            .unwrap();
        match (a, b) {
            (
                pi_core::Entry::Message {
                    seq: 1,
                    parent_id: None,
                    ..
                },
                pi_core::Entry::Message {
                    seq: 2,
                    parent_id: Some(p),
                    ..
                },
            ) => assert_eq!(p, "m1"),
            other => panic!("unexpected {other:?}"),
        }
        repo.create_lane("s", "review").unwrap();
        let c = repo
            .append_message("s", "review", "m3", Role::User, "side")
            .unwrap();
        match c {
            pi_core::Entry::Message {
                seq: 4,
                parent_id: None,
                ..
            } => {}
            other => panic!(
                "lane should start empty and consume seq 4 after lane_move seq 3, got {other:?}"
            ),
        }
    }

    #[test]
    fn rejects_duplicate_ids() {
        let repo = repo();
        repo.create(Some("s"), "/tmp", None, None).unwrap();
        repo.append_message("s", "main", "m1", Role::User, "one")
            .unwrap();
        let err = repo
            .append_message("s", "main", "m1", Role::User, "dup")
            .unwrap_err();
        match err {
            StoreError::Session(e) => assert_eq!(e.code, SessionErrorCode::AlreadyExists),
            other => panic!("{other:?}"),
        }
        assert_eq!(repo.entries("s").unwrap().len(), 1);
    }

    #[test]
    fn name_facts_latest_wins_and_clear() {
        let repo = repo();
        repo.create(Some("s"), "/tmp", None, None).unwrap();
        repo.set_name("s", Some("alpha")).unwrap();
        repo.set_name("s", Some("beta")).unwrap();
        let listed = repo.list(None).unwrap();
        assert_eq!(listed[0].session_name.as_deref(), Some("beta"));
        repo.set_name("s", None).unwrap();
        let listed = repo.list(None).unwrap();
        assert!(listed[0].session_name.is_none());
    }

    #[test]
    fn delete_is_idempotent() {
        let repo = repo();
        repo.create(Some("s"), "/tmp", None, None).unwrap();
        repo.delete("s").unwrap();
        repo.delete("s").unwrap();
        assert!(repo.list(None).unwrap().is_empty());
    }

    #[test]
    fn fork_copies_tree() {
        let repo = repo();
        repo.create(Some("src"), "/tmp", None, None).unwrap();
        repo.append_message("src", "main", "m1", Role::User, "one")
            .unwrap();
        repo.append_message("src", "main", "m2", Role::Assistant, "two")
            .unwrap();
        repo.fork("src", "dst", "/tmp", true).unwrap();
        assert_eq!(repo.entries("dst").unwrap().len(), 2);
        assert_eq!(repo.entries("src").unwrap().len(), 2);
    }

    #[test]
    fn acquire_sql_matches_official_fence_rules() {
        let repo = repo();
        repo.create(Some("s"), "/tmp", None, None).unwrap();
        let first = repo.current_lease("s").unwrap().unwrap();
        assert_eq!(first.fence, 1);

        // Direct SQL acquire while active must fail.
        let inner_lease = {
            let path = tempfile::NamedTempFile::new().unwrap();
            let db = rusqlite::Connection::open(path.path()).unwrap();
            db.execute_batch(crate::schema::INITIAL_SCHEMA).unwrap();
            let now = 1_000i64;
            let first = acquire_writer_lease(&db, "s", "a", now, now + 30_000)
                .unwrap()
                .unwrap();
            assert_eq!(first.fence, 1);
            assert!(acquire_writer_lease(&db, "s", "b", now + 1, now + 31_000)
                .unwrap()
                .is_none());
            let taken = acquire_writer_lease(&db, "s", "b", now + 40_000, now + 70_000)
                .unwrap()
                .unwrap();
            assert_eq!(taken.fence, 2);
            assert_eq!(taken.owner_id, "b");

            let mut stale = WriterLease {
                owner_id: "a".into(),
                fence: 1,
                expires_at_ms: now + 30_000,
            };
            assert!(!renew_writer_lease(&db, "s", &mut stale, now + 40_000, now + 80_000).unwrap());
            first
        };
        assert_eq!(inner_lease.owner_id, "a");
    }

    #[test]
    fn heartbeat_renews_idle_lease() {
        let repo = repo();
        repo.create(Some("s"), "/tmp", None, None).unwrap();
        let before = repo.current_lease("s").unwrap().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(repo.heartbeat("s").unwrap());
        let after = repo.current_lease("s").unwrap().unwrap();
        assert_eq!(after.owner_id, before.owner_id);
        assert_eq!(after.fence, before.fence);
        assert!(after.expires_at_ms >= before.expires_at_ms);
    }
}
