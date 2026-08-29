use std::collections::BTreeMap;

use pi_core::{next_id, SessionError};

use crate::{
    provision_message, EntryQuery, ForkOptions, ForkScope, LaneRecord, LogItem, QueryOrder,
    SessionCreateOptions, SessionRepository,
};

#[derive(Debug, Default)]
pub struct ConformanceReport {
    pub passed: Vec<String>,
    pub failed: Vec<(String, String)>,
}

impl ConformanceReport {
    pub fn ok(&self) -> bool {
        self.failed.is_empty()
    }
}

fn check(report: &mut ConformanceReport, name: &str, run: impl FnOnce() -> Result<(), String>) {
    match run() {
        Ok(()) => report.passed.push(name.to_string()),
        Err(error) => report.failed.push((name.to_string(), error)),
    }
}

/// Shared session-backend conformance matrix (Phase 2).
pub fn run_conformance(repo: &mut dyn SessionRepository) -> ConformanceReport {
    let mut report = ConformanceReport::default();

    check(&mut report, "entries and lanes", || {
        let mut session = repo
            .create(SessionCreateOptions {
                cwd: "/conformance".into(),
                id: Some("entries".into()),
                ..SessionCreateOptions::default()
            })
            .map_err(|error| error.to_string())?;
        let first = session
            .append_entry(provision_message("first"), "main")
            .map_err(err)?;
        let second = session
            .append_entry(provision_message("second"), "main")
            .map_err(err)?;
        if second.parent_id() != Some(first.id()) {
            return Err("lane parent was not assigned from the previous leaf".into());
        }
        let oldest = session
            .find_entries(EntryQuery {
                order: Some(QueryOrder::OldestFirst),
                ..EntryQuery::default()
            })
            .map_err(err)?;
        if oldest.len() != 2 || oldest[0].id() != first.id() {
            return Err("oldestFirst query did not preserve append order".into());
        }
        Ok(())
    });

    check(&mut report, "queries and facts", || {
        let mut session = repo
            .create(SessionCreateOptions {
                cwd: "/conformance".into(),
                id: Some("facts".into()),
                ..SessionCreateOptions::default()
            })
            .map_err(err)?;
        session
            .append_entry(provision_message("named"), "main")
            .map_err(err)?;
        session.set_name(Some("Review session")).map_err(err)?;
        if session.get_name().map_err(err)?.as_deref() != Some("Review session") {
            return Err("setName/getName did not persist".into());
        }
        session.set_name(None).map_err(err)?;
        if session.get_name().map_err(err)?.is_some() {
            return Err("clearing name should omit the projected name".into());
        }
        Ok(())
    });

    check(&mut report, "repository create list open delete", || {
        let mut created = repo
            .create(SessionCreateOptions {
                cwd: "/conformance".into(),
                id: Some("repo-lifecycle".into()),
                metadata: Some(serde_json::json!({"profile":"reviewer"})),
                ..SessionCreateOptions::default()
            })
            .map_err(err)?;
        let meta = created.metadata().map_err(err)?;
        let listed = repo.list(Some("/conformance")).map_err(err)?;
        if !listed.iter().any(|item| item.id == meta.id) {
            return Err("list did not include the created session".into());
        }
        created.release().map_err(err)?;
        let mut opened = repo.open(&meta).map_err(err)?;
        opened
            .append_entry(provision_message("reopen"), "main")
            .map_err(err)?;
        opened.release().map_err(err)?;
        repo.delete(&meta).map_err(err)?;
        if repo.open(&meta).is_ok() {
            return Err("deleted session should not open".into());
        }
        Ok(())
    });

    check(&mut report, "records and log", || {
        let mut session = repo
            .create(SessionCreateOptions {
                cwd: "/conformance".into(),
                id: Some("records".into()),
                ..SessionCreateOptions::default()
            })
            .map_err(err)?;
        session
            .append_entry(provision_message("logged"), "main")
            .map_err(err)?;
        let started = session
            .append_record(LaneRecord::OperationStarted {
                id: next_id(),
                seq: 0,
                lane: "main".into(),
                timestamp: 0,
                run_id: Some("run-1".into()),
                extra: BTreeMap::new(),
            })
            .map_err(err)?;
        let records = session.find_records(Some("main")).map_err(err)?;
        if records.len() != 1 || records[0].id() != started.id() {
            return Err("findRecords did not return the appended lane record".into());
        }
        let log = session.get_log(None).map_err(err)?;
        let has_entry = log.iter().any(|item| matches!(item, LogItem::Entry { .. }));
        let has_record = log
            .iter()
            .any(|item| matches!(item, LogItem::Record { .. }));
        if !has_entry || !has_record {
            return Err(
                "getLog must include both entries and records as separate mutations".into(),
            );
        }
        Ok(())
    });

    check(&mut report, "repository and forks", || {
        let mut source = repo
            .create(SessionCreateOptions {
                cwd: "/conformance".into(),
                id: Some("source".into()),
                name: Some("Source".into()),
                ..SessionCreateOptions::default()
            })
            .map_err(err)?;
        source
            .append_entry(provision_message("root"), "main")
            .map_err(err)?;
        source
            .append_entry(provision_message("child"), "main")
            .map_err(err)?;
        let source_id = source.metadata().map_err(err)?.id;
        let source_count = source
            .find_entries(EntryQuery {
                order: Some(QueryOrder::OldestFirst),
                ..EntryQuery::default()
            })
            .map_err(err)?
            .len();
        let fork = repo
            .fork(
                source.as_ref(),
                ForkOptions {
                    cwd: "/conformance".into(),
                    scope: ForkScope::Branch,
                    position: crate::ForkPosition::At,
                    entry_id: None,
                },
            )
            .map_err(err)?;
        let fork_meta = fork.metadata().map_err(err)?;
        if fork_meta.parent_session_id.as_deref() != Some(source_id.as_str()) {
            return Err("fork must record parentSessionId".into());
        }
        if fork.get_name().map_err(err)?.is_some() {
            return Err("branch fork should not copy the source name fact".into());
        }
        let forked = fork
            .find_entries(EntryQuery {
                order: Some(QueryOrder::OldestFirst),
                ..EntryQuery::default()
            })
            .map_err(err)?;
        if forked.len() != source_count {
            return Err("branch fork did not copy the source entries".into());
        }
        let still_source = source
            .find_entries(EntryQuery::default())
            .map_err(err)?
            .len();
        if still_source != source_count {
            return Err("fork must not mutate the source session".into());
        }
        Ok(())
    });

    check(&mut report, "validation rejects duplicate ids", || {
        let mut session = repo
            .create(SessionCreateOptions {
                cwd: "/conformance".into(),
                id: Some("dupes".into()),
                ..SessionCreateOptions::default()
            })
            .map_err(err)?;
        let first = session
            .append_entry(provision_message("once"), "main")
            .map_err(err)?;
        let mut again = provision_message("twice");
        if let crate::Entry::Message { id, .. } = &mut again {
            *id = first.id().to_string();
        }
        match session.append_entry(again, "main") {
            Err(SessionError { .. }) => Ok(()),
            Ok(_) => Err("duplicate id must be rejected".into()),
        }
    });

    report
}

fn err(error: SessionError) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemorySessionRepository;

    #[test]
    fn memory_backend_passes_conformance() {
        let mut repo = MemorySessionRepository::new("memory");
        let report = run_conformance(&mut repo);
        assert!(report.ok(), "conformance failures: {:?}", report.failed);
    }
}
