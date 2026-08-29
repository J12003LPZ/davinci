//! SQLite session backend with TypeScript-compatible writer-leases.

mod leases;
mod repo;

pub use leases::{
    acquire_writer_lease, delete_writer_lease, release_writer_lease, renew_writer_lease, WriterLease,
    WriterLeaseOptions,
};
pub use repo::{apply_migrations, open_database, SqliteSessionRepository, SqliteSessionStorage};

#[cfg(test)]
mod tests {
    use pi_core::SessionErrorCode;
    use pi_session::{
        provision_message, run_conformance, EntryQuery, QueryOrder, SessionCreateOptions, SessionRepository,
    };
    use tempfile::tempdir;

    use super::*;

    fn repo(dir: &std::path::Path) -> SqliteSessionRepository {
        SqliteSessionRepository::open_default(dir.join("sessions.sqlite")).unwrap()
    }

    #[test]
    fn writer_lease_options_match_typescript_validation() {
        let err = WriterLeaseOptions::resolve(Some(0), Some(1)).unwrap_err();
        assert_eq!(err.message, "writerLease.ttlMs must be positive");
        let err = WriterLeaseOptions::resolve(Some(100), Some(100)).unwrap_err();
        assert_eq!(
            err.message,
            "writerLease.heartbeatIntervalMs must be positive and less than ttlMs"
        );
    }

    #[test]
    fn rejects_second_writer_until_release() {
        let dir = tempdir().unwrap();
        let mut first = repo(dir.path());
        let mut second = repo(dir.path());
        let session = first
            .create(SessionCreateOptions {
                cwd: dir.path().to_string_lossy().into_owned(),
                id: Some("session-1".into()),
                ..SessionCreateOptions::default()
            })
            .unwrap();
        let metadata = session.metadata().unwrap();
        let err = match second.open(&metadata) {
            Err(error) => error,
            Ok(_) => panic!("expected second writer to fail"),
        };
        assert_eq!(err.code, SessionErrorCode::Storage);
        assert!(err.message.contains("already has an active writer"));

        let mut owned = session;
        owned.release().unwrap();
        let mut reopened = second.open(&metadata).unwrap();
        reopened
            .append_entry(provision_message("new owner"), "main")
            .unwrap();
    }

    #[test]
    fn fences_stale_owner_after_expiry() {
        let dir = tempdir().unwrap();
        let options = WriterLeaseOptions {
            ttl_ms: 120_000,
            heartbeat_interval_ms: 60_000,
        };
        let mut first = SqliteSessionRepository::open(dir.path().join("sessions.sqlite"), options).unwrap();
        let mut second = SqliteSessionRepository::open(dir.path().join("sessions.sqlite"), options).unwrap();
        let mut stale = first
            .create(SessionCreateOptions {
                cwd: dir.path().to_string_lossy().into_owned(),
                id: Some("session-1".into()),
                ..SessionCreateOptions::default()
            })
            .unwrap();
        let metadata = stale.metadata().unwrap();
        {
            let db = rusqlite::Connection::open(dir.path().join("sessions.sqlite")).unwrap();
            db.execute(
                "UPDATE writer_leases SET expires_at_ms = 0 WHERE session_id = ?1",
                rusqlite::params![metadata.id],
            )
            .unwrap();
        }

        let mut current = second.open(&metadata).unwrap();
        let err = stale.append_entry(provision_message("stale owner"), "main").unwrap_err();
        assert!(err.message.contains("writer lease was lost"));
        assert!(current.find_entries(EntryQuery::default()).unwrap().is_empty());
        let fence = second
            .inspect_leases()
            .unwrap()
            .into_iter()
            .find(|(id, _, _, _)| id == &metadata.id)
            .map(|(_, _, fence, _)| fence)
            .unwrap();
        assert_eq!(fence, 2);
        current
            .append_entry(provision_message("current owner"), "main")
            .unwrap();
    }

    #[test]
    fn list_does_not_mutate_writer_leases() {
        let dir = tempdir().unwrap();
        let mut writer = repo(dir.path());
        let reader = repo(dir.path());
        let mut first = writer
            .create(SessionCreateOptions {
                cwd: dir.path().to_string_lossy().into_owned(),
                id: Some("session-1".into()),
                metadata: Some(serde_json::json!({"profile":"reviewer"})),
                ..SessionCreateOptions::default()
            })
            .unwrap();
        first.set_name(Some("Review session")).unwrap();
        let before = writer.inspect_leases().unwrap();
        let listed = reader
            .list(Some(&dir.path().to_string_lossy()))
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name.as_deref(), Some("Review session"));
        assert_eq!(writer.inspect_leases().unwrap(), before);
    }

    #[test]
    fn shared_reopen_in_one_process_is_serialized_by_lease_sql() {
        let dir = tempdir().unwrap();
        let mut repository = repo(dir.path());
        let mut session = repository
            .create(SessionCreateOptions {
                cwd: dir.path().to_string_lossy().into_owned(),
                id: Some("session".into()),
                ..SessionCreateOptions::default()
            })
            .unwrap();
        let first = session.append_entry(provision_message("first"), "main").unwrap();
        let second = session.append_entry(provision_message("second"), "main").unwrap();
        let ids: Vec<_> = session
            .find_entries(EntryQuery {
                order: Some(QueryOrder::OldestFirst),
                ..EntryQuery::default()
            })
            .unwrap()
            .into_iter()
            .map(|entry| entry.id().to_string())
            .collect();
        assert_eq!(ids, vec![first.id().to_string(), second.id().to_string()]);
    }

    #[test]
    fn sqlite_backend_passes_session_conformance() {
        let dir = tempdir().unwrap();
        let mut repository = repo(dir.path());
        let report = run_conformance(&mut repository);
        assert!(report.ok(), "conformance failures: {:?}", report.failed);
    }
}

