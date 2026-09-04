//! TypeScript `createSessionBackendConformance` cases, locked to the same codes and shapes.

use crate::backend::{
    assistant_message, operation_started, user_message, BackendError, BackendErrorCode,
    CreateOptions, EntryQuery, ForkOptions, ForkPosition, ForkScope, LogItem, LogOptions,
    RecordQuery, Session, SessionRepository, SessionStats,
};
use serde_json::{json, Value};

fn expect_code<T>(result: Result<T, BackendError>, code: BackendErrorCode) -> Result<(), String> {
    match result {
        Ok(_) => Err(format!("expected {code:?}")),
        Err(err) if err.code == code => Ok(()),
        Err(err) => Err(format!(
            "expected {code:?}, got {:?} ({})",
            err.code, err.message
        )),
    }
}

fn ids(entries: &[Value]) -> Vec<String> {
    entries
        .iter()
        .filter_map(|e| e.get("id").and_then(Value::as_str).map(str::to_string))
        .collect()
}

fn log_kinds(session: &Session) -> Result<Vec<(String, u64)>, String> {
    Ok(session
        .get_log(LogOptions::default())
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|item| match item {
            LogItem::Entry { seq, .. } => ("entry".into(), seq),
            LogItem::Record { seq, .. } => ("record".into(), seq),
            LogItem::Lane { seq, .. } => ("lane".into(), seq),
            LogItem::Fact { seq, .. } => ("fact".into(), seq),
        })
        .collect())
}

fn log_seqs(session: &Session) -> Result<Vec<u64>, String> {
    Ok(log_kinds(session)?.into_iter().map(|(_, s)| s).collect())
}

fn record_ids(records: &[Value]) -> Vec<String> {
    records
        .iter()
        .filter_map(|r| r.get("id").and_then(Value::as_str).map(str::to_string))
        .collect()
}

fn ts(value: &Value) -> Option<u64> {
    value.get("timestamp").and_then(Value::as_u64)
}

pub fn run_all<R, F>(mut factory: F) -> Result<(), String>
where
    R: SessionRepository,
    F: FnMut() -> R,
{
    type Case<R> = fn(&R) -> Result<(), String>;
    let cases: &[(&str, Case<R>)] = &[
        (
            "assigns parents and one sequence across every mutation",
            assigns_parents_and_sequence,
        ),
        (
            "commits records and lane moves as separate mutations",
            commits_records_and_lane_moves,
        ),
        (
            "rejects duplicate ids without changing state",
            rejects_duplicate_ids,
        ),
        ("isolates lanes while sharing the tree", isolates_lanes),
        (
            "rejects invalid queries before empty reads",
            rejects_invalid_queries,
        ),
        (
            "supports bounded filtered and cursor-based queries",
            bounded_filtered_queries,
        ),
        (
            "keeps lane names permanent with their recovery records",
            keeps_lane_names,
        ),
        (
            "persists queue cancellation without consuming its target",
            queue_cancellation,
        ),
        (
            "filters records by lane type run sequence and order",
            filters_records,
        ),
        (
            "filters operation starts by operation kind",
            filters_operation_kind,
        ),
        (
            "tracks and enforces one open operation per lane",
            one_open_operation,
        ),
        (
            "does not let an earlier finish close a later start",
            earlier_finish,
        ),
        (
            "scopes open operations by lane and limit",
            scopes_open_operations,
        ),
        (
            "returns immutable open-operation records",
            immutable_open_ops,
        ),
        (
            "keeps latest-value facts and computes ledger statistics across lanes",
            facts_and_stats,
        ),
        ("clears session names durably", clears_names),
        ("returns immutable copies from reads", immutable_reads),
        ("validates lane lifecycle and targets", lane_lifecycle),
        ("binds lane views without caching leaves", lane_views),
        (
            "appends provisioned entries with their existing ids",
            provisioned_ids,
        ),
        (
            "persists tool-result termination decisions",
            tool_result_terminate,
        ),
        (
            "rejects non-JSON entries before storage mutation",
            rejects_non_json_entries,
        ),
        (
            "rejects non-JSON records before storage mutation",
            rejects_non_json_records,
        ),
        (
            "linearizes concurrent writes across two lanes",
            concurrent_writes,
        ),
        ("creates lists and opens sessions", create_list_open),
        ("deletes sessions idempotently", delete_idempotent),
        (
            "forks one branch with selected facts and no records",
            fork_branch,
        ),
        ("forks a complete tree with lanes and facts", fork_tree),
        (
            "forks before an entry without modifying the source",
            fork_before,
        ),
        ("validates the default fork target", fork_default_target),
    ];
    for (name, test) in cases {
        let repo = factory();
        test(&repo).map_err(|e| format!("{name}: {e}"))?;
    }
    Ok(())
}

fn assigns_parents_and_sequence<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let session = repo
        .create(CreateOptions {
            id: Some("session".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    let root = session
        .append_entry(
            json!({"type":"message","id":"root","message": user_message("root")}),
            "main",
        )
        .map_err(|e| e.to_string())?;
    session
        .create_lane("thread", Some("root"))
        .map_err(|e| e.to_string())?;
    let child = session
        .append_entry(
            json!({"type":"custom","id":"child","customType":"note","data":{"value":1}}),
            "thread",
        )
        .map_err(|e| e.to_string())?;
    let record = session
        .append_record(operation_started("run", "thread", "run"))
        .map_err(|e| e.to_string())?;
    session
        .set_name(Some("Example"))
        .map_err(|e| e.to_string())?;
    session
        .set_label("root", Some("checkpoint"))
        .map_err(|e| e.to_string())?;
    session
        .move_lane("main", Some("child"))
        .map_err(|e| e.to_string())?;

    assert_eq!(root.get("parentId"), Some(&Value::Null));
    assert_eq!(root.get("seq"), Some(&json!(1)));
    assert_eq!(child.get("parentId"), Some(&json!("root")));
    assert_eq!(child.get("seq"), Some(&json!(3)));
    assert_eq!(record.get("seq"), Some(&json!(4)));
    for stamp in [ts(&root), ts(&child), ts(&record)] {
        let stamp = stamp.ok_or("missing timestamp")?;
        if stamp > (i64::MAX as u64) {
            return Err("timestamp not a safe integer".into());
        }
    }
    assert_eq!(
        log_kinds(&session)?,
        vec![
            ("entry".into(), 1),
            ("lane".into(), 2),
            ("entry".into(), 3),
            ("record".into(), 4),
            ("fact".into(), 5),
            ("fact".into(), 6),
            ("lane".into(), 7),
        ]
    );
    let lanes = session.get_lanes().map_err(|e| e.to_string())?;
    assert_eq!(lanes[0].lane, "main");
    assert_eq!(lanes[0].leaf_id.as_deref(), Some("child"));
    assert_eq!(lanes[1].lane, "thread");
    assert_eq!(lanes[1].leaf_id.as_deref(), Some("child"));
    Ok(())
}

fn commits_records_and_lane_moves<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let session = repo
        .create(CreateOptions {
            id: Some("session".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    session
        .append_entry(
            json!({"type":"message","id":"root","message": user_message("root")}),
            "main",
        )
        .map_err(|e| e.to_string())?;
    let finished = session
        .append_record(json!({
            "type":"operation_finished","id":"finish","lane":"main","runId":"run","outcome":"completed"
        }))
        .map_err(|e| e.to_string())?;
    assert_eq!(finished.get("seq"), Some(&json!(2)));
    let lanes = session.get_lanes().map_err(|e| e.to_string())?;
    assert_eq!(lanes[0].leaf_id.as_deref(), Some("root"));
    session.move_lane("main", None).map_err(|e| e.to_string())?;
    let lanes = session.get_lanes().map_err(|e| e.to_string())?;
    assert_eq!(lanes[0].leaf_id, None);
    expect_code(
        session.move_lane("main", Some("missing")),
        BackendErrorCode::NotFound,
    )?;
    assert_eq!(
        session
            .find_records(RecordQuery::default())
            .map_err(|e| e.to_string())?
            .len(),
        1
    );
    assert_eq!(log_seqs(&session)?, vec![1, 2, 3]);
    Ok(())
}

fn rejects_duplicate_ids<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let session = repo
        .create(CreateOptions {
            id: Some("session".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    session
        .append_entry(
            json!({"type":"message","id":"shared","message": user_message("root")}),
            "main",
        )
        .map_err(|e| e.to_string())?;
    expect_code(
        session.append_record(operation_started("shared", "main", "run")),
        BackendErrorCode::AlreadyExists,
    )?;
    session
        .append_record(operation_started("run", "main", "run"))
        .map_err(|e| e.to_string())?;
    expect_code(
        session.append_entry(
            json!({"type":"custom","id":"run","customType":"note"}),
            "main",
        ),
        BackendErrorCode::AlreadyExists,
    )?;
    assert_eq!(log_seqs(&session)?, vec![1, 2]);
    Ok(())
}

fn isolates_lanes<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let session = repo
        .create(CreateOptions {
            id: Some("session".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    session
        .append_entry(
            json!({"type":"message","id":"root","message": user_message("root")}),
            "main",
        )
        .map_err(|e| e.to_string())?;
    session
        .create_lane("thread", Some("root"))
        .map_err(|e| e.to_string())?;
    session
        .append_entry(
            json!({"type":"message","id":"main-child","message": user_message("main")}),
            "main",
        )
        .map_err(|e| e.to_string())?;
    session
        .append_entry(
            json!({"type":"message","id":"thread-child","message": user_message("thread")}),
            "thread",
        )
        .map_err(|e| e.to_string())?;
    let lanes = session.get_lanes().map_err(|e| e.to_string())?;
    assert_eq!(lanes[0].leaf_id.as_deref(), Some("main-child"));
    assert_eq!(lanes[1].leaf_id.as_deref(), Some("thread-child"));
    assert_eq!(
        ids(&session
            .find_entries_on_branch(
                "main",
                EntryQuery {
                    start: Some("main-child".into()),
                    order: Some("oldestFirst".into()),
                    ..EntryQuery::default()
                }
            )
            .map_err(|e| e.to_string())?),
        vec!["root".to_string(), "main-child".into()]
    );
    assert_eq!(
        ids(&session
            .find_entries_on_branch(
                "thread",
                EntryQuery {
                    start: Some("thread-child".into()),
                    order: Some("oldestFirst".into()),
                    ..EntryQuery::default()
                }
            )
            .map_err(|e| e.to_string())?),
        vec!["root".to_string(), "thread-child".into()]
    );
    Ok(())
}

fn rejects_invalid_queries<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let session = repo
        .create(CreateOptions {
            id: Some("invalid-queries".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    session
        .create_lane("thread", None)
        .map_err(|e| e.to_string())?;
    expect_code(
        session.find_entries(EntryQuery {
            limit: Some(0),
            ..EntryQuery::default()
        }),
        BackendErrorCode::InvalidQuery,
    )?;
    expect_code(
        session.find_entry(EntryQuery {
            limit: Some(0),
            ..EntryQuery::default()
        }),
        BackendErrorCode::InvalidQuery,
    )?;
    expect_code(
        session.find_entries_on_branch(
            "main",
            EntryQuery {
                limit: Some(0),
                ..EntryQuery::default()
            },
        ),
        BackendErrorCode::InvalidQuery,
    )?;
    expect_code(
        session.find_entries_on_branch(
            "thread",
            EntryQuery {
                after_seq: Some(-1),
                ..EntryQuery::default()
            },
        ),
        BackendErrorCode::InvalidQuery,
    )?;
    expect_code(
        session.find_entry_on_branch(
            "thread",
            EntryQuery {
                limit: Some(0),
                ..EntryQuery::default()
            },
        ),
        BackendErrorCode::InvalidQuery,
    )?;
    expect_code(
        session.find_records(RecordQuery {
            limit: Some(0),
            ..RecordQuery::default()
        }),
        BackendErrorCode::InvalidQuery,
    )?;
    expect_code(
        session.find_records(RecordQuery {
            operation_kind: Some("run".into()),
            ..RecordQuery::default()
        }),
        BackendErrorCode::InvalidQuery,
    )?;
    expect_code(
        session.find_records(RecordQuery {
            type_name: Some("step_attempt".into()),
            operation_kind: Some("run".into()),
            ..RecordQuery::default()
        }),
        BackendErrorCode::InvalidQuery,
    )?;
    expect_code(
        session.find_open_operations("main", Some(0)),
        BackendErrorCode::InvalidQuery,
    )?;
    expect_code(
        session.find_open_operations("main", Some(-1)),
        BackendErrorCode::InvalidQuery,
    )?;
    expect_code(
        session.get_log(LogOptions {
            after_seq: Some(-1),
            ..LogOptions::default()
        }),
        BackendErrorCode::InvalidQuery,
    )?;
    Ok(())
}

fn bounded_filtered_queries<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let session = repo
        .create(CreateOptions {
            id: Some("session".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    session
        .append_entry(
            json!({"type":"message","id":"root","message": user_message("root")}),
            "main",
        )
        .map_err(|e| e.to_string())?;
    session
        .append_entry(
            json!({"type":"custom","id":"old-note","customType":"note","data":1}),
            "main",
        )
        .map_err(|e| e.to_string())?;
    session
        .append_entry(
            json!({"type":"compaction","id":"compact","summary":"summary","retainedTail":[],"tokensBefore":10}),
            "main",
        )
        .map_err(|e| e.to_string())?;
    session
        .append_entry(
            json!({"type":"custom","id":"new-note","customType":"note","data":2}),
            "main",
        )
        .map_err(|e| e.to_string())?;
    session
        .append_entry(
            json!({"type":"message","id":"tail","message": assistant_message("tail")}),
            "main",
        )
        .map_err(|e| e.to_string())?;
    assert_eq!(
        ids(&session
            .find_entries(EntryQuery::default())
            .map_err(|e| e.to_string())?),
        vec!["tail", "new-note", "compact", "old-note", "root"]
    );
    assert_eq!(
        ids(&session
            .find_entries(EntryQuery {
                order: Some("oldestFirst".into()),
                after_seq: Some(2),
                limit: Some(2),
                ..EntryQuery::default()
            })
            .map_err(|e| e.to_string())?),
        vec!["compact", "new-note"]
    );
    assert_eq!(
        ids(&session
            .find_entries(EntryQuery {
                custom_type: Some("note".into()),
                ..EntryQuery::default()
            })
            .map_err(|e| e.to_string())?),
        vec!["new-note", "old-note"]
    );
    assert_eq!(
        ids(&session
            .find_entries_on_branch(
                "main",
                EntryQuery {
                    start: Some("tail".into()),
                    custom_type: Some("note".into()),
                    limit: Some(1),
                    ..EntryQuery::default()
                }
            )
            .map_err(|e| e.to_string())?),
        vec!["new-note"]
    );
    assert_eq!(
        ids(&session
            .find_entries_on_branch(
                "main",
                EntryQuery {
                    start: Some("tail".into()),
                    stop_at_type: Some("compaction".into()),
                    type_name: Some("message".into()),
                    ..EntryQuery::default()
                }
            )
            .map_err(|e| e.to_string())?),
        vec!["tail"]
    );
    assert_eq!(
        ids(&session
            .find_entries_on_branch(
                "main",
                EntryQuery {
                    start: Some("tail".into()),
                    stop_at_id: Some("tail".into()),
                    type_name: Some("custom".into()),
                    ..EntryQuery::default()
                }
            )
            .map_err(|e| e.to_string())?),
        Vec::<String>::new()
    );
    assert_eq!(
        ids(&session
            .find_entries_on_branch(
                "main",
                EntryQuery {
                    start: Some("tail".into()),
                    stop_at_type: Some("custom".into()),
                    order: Some("oldestFirst".into()),
                    ..EntryQuery::default()
                }
            )
            .map_err(|e| e.to_string())?),
        vec!["root", "old-note"]
    );
    expect_code(
        session.find_entries(EntryQuery {
            limit: Some(0),
            ..EntryQuery::default()
        }),
        BackendErrorCode::InvalidQuery,
    )?;
    expect_code(
        session.find_entries_on_branch(
            "main",
            EntryQuery {
                start: Some("missing".into()),
                ..EntryQuery::default()
            },
        ),
        BackendErrorCode::NotFound,
    )?;
    Ok(())
}

fn keeps_lane_names<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let session = repo
        .create(CreateOptions {
            id: Some("session".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    session
        .create_lane("thread", None)
        .map_err(|e| e.to_string())?;
    session
        .append_record(operation_started("old-run", "thread", "run"))
        .map_err(|e| e.to_string())?;
    session
        .append_record(json!({
            "type":"queue_enqueued",
            "id":"old-next-run",
            "lane":"thread",
            "queue":"nextRun",
            "target":{"type":"message","id":"queued-message","message": user_message("queued")}
        }))
        .map_err(|e| e.to_string())?;
    assert_eq!(
        record_ids(
            &session
                .find_records(RecordQuery {
                    lane: Some("thread".into()),
                    ..RecordQuery::default()
                })
                .map_err(|e| e.to_string())?
        ),
        vec!["old-next-run", "old-run"]
    );
    expect_code(
        session.create_lane("thread", None),
        BackendErrorCode::AlreadyExists,
    )?;
    Ok(())
}

fn queue_cancellation<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let session = repo
        .create(CreateOptions {
            id: Some("session".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    session
        .append_record(json!({
            "type":"queue_enqueued",
            "id":"enqueue",
            "lane":"main",
            "queue":"nextRun",
            "target":{"type":"message","id":"queued-message","message": user_message("queued")}
        }))
        .map_err(|e| e.to_string())?;
    let cancelled = session
        .append_record(json!({
            "type":"queue_cancelled","id":"cancel","lane":"main","entryId":"queued-message"
        }))
        .map_err(|e| e.to_string())?;
    assert_eq!(cancelled.get("seq"), Some(&json!(2)));
    assert_eq!(cancelled.get("entryId"), Some(&json!("queued-message")));
    assert!(cancelled.get("runId").is_none());
    assert!(session
        .get_entry("queued-message")
        .map_err(|e| e.to_string())?
        .is_none());
    let found = session
        .find_records(RecordQuery {
            type_name: Some("queue_cancelled".into()),
            ..RecordQuery::default()
        })
        .map_err(|e| e.to_string())?;
    assert_eq!(found[0].get("entryId"), Some(&json!("queued-message")));
    Ok(())
}

fn filters_records<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let session = repo
        .create(CreateOptions {
            id: Some("session".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    session
        .append_record(operation_started("run-1", "main", "run"))
        .map_err(|e| e.to_string())?;
    session
        .append_record(json!({
            "type":"step_attempt","id":"attempt-1","lane":"main","runId":"run-1",
            "step":"assistant","attempt":1,"resultEntryId":"assistant-1"
        }))
        .map_err(|e| e.to_string())?;
    session
        .create_lane("thread", None)
        .map_err(|e| e.to_string())?;
    session
        .append_record(operation_started("run-2", "thread", "run"))
        .map_err(|e| e.to_string())?;
    session
        .append_record(json!({
            "type":"step_attempt","id":"attempt-2","lane":"thread","runId":"run-2",
            "step":"assistant","attempt":1,"resultEntryId":"assistant-2"
        }))
        .map_err(|e| e.to_string())?;
    assert_eq!(
        record_ids(
            &session
                .find_records(RecordQuery {
                    lane: Some("thread".into()),
                    ..RecordQuery::default()
                })
                .map_err(|e| e.to_string())?
        ),
        vec!["attempt-2", "run-2"]
    );
    assert_eq!(
        record_ids(
            &session
                .find_records(RecordQuery {
                    type_name: Some("step_attempt".into()),
                    order: Some("oldestFirst".into()),
                    ..RecordQuery::default()
                })
                .map_err(|e| e.to_string())?
        ),
        vec!["attempt-1", "attempt-2"]
    );
    assert_eq!(
        record_ids(
            &session
                .find_records(RecordQuery {
                    run_id: Some("run-1".into()),
                    after_seq: Some(1),
                    ..RecordQuery::default()
                })
                .map_err(|e| e.to_string())?
        ),
        vec!["attempt-1"]
    );
    assert_eq!(
        record_ids(
            &session
                .find_records(RecordQuery {
                    limit: Some(1),
                    ..RecordQuery::default()
                })
                .map_err(|e| e.to_string())?
        ),
        vec!["attempt-2"]
    );
    Ok(())
}

fn filters_operation_kind<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let session = repo
        .create(CreateOptions {
            id: Some("session".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    session
        .append_record(operation_started("run-old", "main", "run"))
        .map_err(|e| e.to_string())?;
    session
        .append_record(json!({
            "type":"operation_finished","id":"run-old-finished","lane":"main","runId":"run-old","outcome":"completed"
        }))
        .map_err(|e| e.to_string())?;
    session
        .append_record(operation_started("compaction", "main", "compaction"))
        .map_err(|e| e.to_string())?;
    session
        .append_record(json!({
            "type":"operation_finished","id":"compaction-finished","lane":"main","runId":"compaction","outcome":"completed"
        }))
        .map_err(|e| e.to_string())?;
    session
        .append_record(operation_started("navigation", "main", "navigation"))
        .map_err(|e| e.to_string())?;
    session
        .append_record(json!({
            "type":"operation_finished","id":"navigation-finished","lane":"main","runId":"navigation","outcome":"completed"
        }))
        .map_err(|e| e.to_string())?;
    session
        .append_record(operation_started("run-new", "main", "run"))
        .map_err(|e| e.to_string())?;
    assert_eq!(
        record_ids(
            &session
                .find_records(RecordQuery {
                    type_name: Some("operation_started".into()),
                    operation_kind: Some("run".into()),
                    order: Some("oldestFirst".into()),
                    ..RecordQuery::default()
                })
                .map_err(|e| e.to_string())?
        ),
        vec!["run-old", "run-new"]
    );
    assert_eq!(
        record_ids(
            &session
                .find_records(RecordQuery {
                    type_name: Some("operation_started".into()),
                    operation_kind: Some("compaction".into()),
                    ..RecordQuery::default()
                })
                .map_err(|e| e.to_string())?
        ),
        vec!["compaction"]
    );
    assert_eq!(
        record_ids(
            &session
                .find_records(RecordQuery {
                    type_name: Some("operation_started".into()),
                    operation_kind: Some("run".into()),
                    limit: Some(1),
                    ..RecordQuery::default()
                })
                .map_err(|e| e.to_string())?
        ),
        vec!["run-new"]
    );
    Ok(())
}

fn one_open_operation<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let session = repo
        .create(CreateOptions {
            id: Some("session".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    assert!(session
        .find_open_operations("main", Some(2))
        .map_err(|e| e.to_string())?
        .is_empty());
    let first = session
        .append_record(operation_started("first", "main", "run"))
        .map_err(|e| e.to_string())?;
    assert_eq!(
        session
            .find_open_operations("main", Some(2))
            .map_err(|e| e.to_string())?
            .len(),
        1
    );
    expect_code(
        session.append_record(operation_started("second", "main", "run")),
        BackendErrorCode::Storage,
    )?;
    session
        .append_record(json!({
            "type":"operation_finished","id":"finish-first","lane":"main","runId": first.get("id").unwrap(),"outcome":"completed"
        }))
        .map_err(|e| e.to_string())?;
    assert!(session
        .find_open_operations("main", Some(2))
        .map_err(|e| e.to_string())?
        .is_empty());
    Ok(())
}

fn earlier_finish<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let session = repo
        .create(CreateOptions {
            id: Some("session".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    session
        .append_record(json!({
            "type":"operation_finished","id":"finish-before-start","lane":"main","runId":"run","outcome":"completed"
        }))
        .map_err(|e| e.to_string())?;
    let started = session
        .append_record(operation_started("run", "main", "run"))
        .map_err(|e| e.to_string())?;
    let open = session
        .find_open_operations("main", Some(2))
        .map_err(|e| e.to_string())?;
    assert_eq!(open[0].get("id"), started.get("id"));
    Ok(())
}

fn scopes_open_operations<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let session = repo
        .create(CreateOptions {
            id: Some("session".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    session
        .create_lane("thread", None)
        .map_err(|e| e.to_string())?;
    let main_run = session
        .append_record(operation_started("main-run", "main", "run"))
        .map_err(|e| e.to_string())?;
    let thread_nav = session
        .append_record(operation_started(
            "thread-navigation",
            "thread",
            "navigation",
        ))
        .map_err(|e| e.to_string())?;
    assert_eq!(
        session
            .find_open_operations("main", None)
            .map_err(|e| e.to_string())?[0]
            .get("id"),
        main_run.get("id")
    );
    assert_eq!(
        session
            .find_open_operations("thread", Some(2))
            .map_err(|e| e.to_string())?[0]
            .get("id"),
        thread_nav.get("id")
    );
    Ok(())
}

fn immutable_open_ops<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let session = repo
        .create(CreateOptions {
            id: Some("session".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    let committed = session
        .append_record(operation_started("run", "main", "run"))
        .map_err(|e| e.to_string())?;
    let mut read = session
        .find_open_operations("main", None)
        .map_err(|e| e.to_string())?;
    if let Some(obj) = read[0]
        .pointer_mut("/intent/originalPrompt")
        .and_then(Value::as_array_mut)
    {
        obj.push(user_message("mutated"));
    }
    let again = session
        .find_open_operations("main", None)
        .map_err(|e| e.to_string())?;
    assert_eq!(again[0].get("id"), committed.get("id"));
    assert_eq!(again[0].pointer("/intent/originalPrompt"), Some(&json!([])));
    Ok(())
}

fn facts_and_stats<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let session = repo
        .create(CreateOptions {
            id: Some("session".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    let mut assistant = assistant_message("answer");
    assistant["usage"] = json!({
        "input": 10, "output": 5, "cacheRead": 3, "cacheWrite": 2, "totalTokens": 20,
        "cost": {"input": 1, "output": 2, "cacheRead": 3, "cacheWrite": 4, "total": 10}
    });
    session
        .append_entry(
            json!({"type":"message","id":"user","message": user_message("question")}),
            "main",
        )
        .map_err(|e| e.to_string())?;
    session
        .append_entry(
            json!({"type":"message","id":"assistant","message": assistant}),
            "main",
        )
        .map_err(|e| e.to_string())?;
    session
        .append_record(json!({
            "type":"usage","id":"assistant-usage","lane":"main","cause":"assistant",
            "runId":"run","entryId":"assistant","attempt":1,"stopReason":"stop",
            "usage": assistant["usage"]
        }))
        .map_err(|e| e.to_string())?;
    session
        .append_record(json!({
            "type":"usage","id":"deferred-usage","lane":"main","cause":"deferred_fetch",
            "runId":"run","entryId":"deferred-result","attempt":1,"stopReason":"deferred",
            "usage": {
                "input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,
                "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}
            }
        }))
        .map_err(|e| e.to_string())?;
    session
        .create_lane("thread", Some("assistant"))
        .map_err(|e| e.to_string())?;
    session
        .append_record(json!({
            "type":"usage","id":"correction","lane":"thread","cause":"adjustment",
            "details":{"reason":"provider correction"},
            "usage": {
                "input":-2,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":-2,
                "cost":{"input":-0.5,"output":0,"cacheRead":0,"cacheWrite":0,"total":-0.5}
            }
        }))
        .map_err(|e| e.to_string())?;
    session.set_name(Some("First")).map_err(|e| e.to_string())?;
    session
        .set_name(Some("Second"))
        .map_err(|e| e.to_string())?;
    session
        .set_label("user", Some("keep"))
        .map_err(|e| e.to_string())?;
    session.set_label("user", None).map_err(|e| e.to_string())?;
    expect_code(
        session.set_label("missing", Some("checkpoint")),
        BackendErrorCode::NotFound,
    )?;
    assert_eq!(
        session.get_name().map_err(|e| e.to_string())?.as_deref(),
        Some("Second")
    );
    assert_eq!(session.get_label("user").map_err(|e| e.to_string())?, None);
    let usage = session
        .find_records(RecordQuery {
            type_name: Some("usage".into()),
            order: Some("oldestFirst".into()),
            ..RecordQuery::default()
        })
        .map_err(|e| e.to_string())?;
    assert_eq!(
        usage
            .iter()
            .map(|r| r.get("cause").and_then(Value::as_str).unwrap_or(""))
            .collect::<Vec<_>>(),
        vec!["assistant", "deferred_fetch", "adjustment"]
    );
    assert_eq!(usage[1].get("stopReason"), Some(&json!("deferred")));
    let stats = session.get_stats().map_err(|e| e.to_string())?;
    assert_eq!(
        stats,
        SessionStats {
            message_count: 2,
            cached_tokens: 3.0,
            uncached_tokens: 10.0,
            total_tokens: 18.0,
            cost_total: 9.5,
        }
    );
    Ok(())
}

fn clears_names<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let session = repo
        .create(CreateOptions {
            id: Some("session".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    session
        .set_name(Some("Temporary"))
        .map_err(|e| e.to_string())?;
    session.set_name(None).map_err(|e| e.to_string())?;
    assert_eq!(session.get_name().map_err(|e| e.to_string())?, None);
    let log = session
        .get_log(LogOptions::default())
        .map_err(|e| e.to_string())?;
    match &log[0] {
        LogItem::Fact {
            fact, name, seq, ..
        } => {
            assert_eq!(fact, "name");
            assert_eq!(name.as_deref(), Some("Temporary"));
            assert_eq!(*seq, 1);
        }
        _ => return Err("expected name fact".into()),
    }
    match &log[1] {
        LogItem::Fact {
            fact, name, seq, ..
        } => {
            assert_eq!(fact, "name");
            assert_eq!(*name, None);
            assert_eq!(*seq, 2);
        }
        _ => return Err("expected cleared name fact".into()),
    }
    let metadata = session.get_metadata().map_err(|e| e.to_string())?;
    let reopened = repo.open(&metadata).map_err(|e| e.to_string())?;
    assert_eq!(reopened.get_name().map_err(|e| e.to_string())?, None);
    let fork = repo
        .fork(
            &metadata,
            ForkOptions {
                id: Some("fork".into()),
                ..ForkOptions::default()
            },
        )
        .map_err(|e| e.to_string())?;
    assert_eq!(fork.get_name().map_err(|e| e.to_string())?, None);
    Ok(())
}

fn immutable_reads<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let session = repo
        .create(CreateOptions {
            id: Some("immutable".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    let metadata = session.get_metadata().map_err(|e| e.to_string())?;
    session
        .append_entry(
            json!({"type":"custom","id":"custom","customType":"note","data":{"nested":{"value":1}}}),
            "main",
        )
        .map_err(|e| e.to_string())?;
    let mut read = session
        .get_entry("custom")
        .map_err(|e| e.to_string())?
        .unwrap();
    if let Some(v) = read.pointer_mut("/data/nested/value") {
        *v = json!(99);
    }
    let mut read_meta = session.get_metadata().map_err(|e| e.to_string())?;
    read_meta.id = "changed".into();
    let mut log = session
        .get_log(LogOptions::default())
        .map_err(|e| e.to_string())?;
    if let LogItem::Entry { entry, .. } = &mut log[0] {
        if let Some(v) = entry.pointer_mut("/data/nested/value") {
            *v = json!(100);
        }
    }
    assert_eq!(
        session.get_metadata().map_err(|e| e.to_string())?.id,
        metadata.id
    );
    let stored = session
        .get_entry("custom")
        .map_err(|e| e.to_string())?
        .unwrap();
    assert_eq!(stored.pointer("/data/nested/value"), Some(&json!(1)));
    Ok(())
}

fn lane_lifecycle<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let session = repo
        .create(CreateOptions {
            id: Some("session".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    expect_code(
        session.create_lane("main", None),
        BackendErrorCode::AlreadyExists,
    )?;
    expect_code(
        session.create_lane("thread", Some("missing")),
        BackendErrorCode::NotFound,
    )?;
    expect_code(
        session.move_lane("missing", None),
        BackendErrorCode::InvalidLane,
    )?;
    Ok(())
}

fn lane_views<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let session = repo
        .create(CreateOptions {
            id: Some("session".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    let root = session
        .append_message(user_message("root"), "main")
        .map_err(|e| e.to_string())?;
    session
        .create_lane("thread", Some(&root))
        .map_err(|e| e.to_string())?;
    let main_child = session
        .append_message(user_message("main"), "main")
        .map_err(|e| e.to_string())?;
    let thread_child = session
        .append_message(user_message("thread"), "thread")
        .map_err(|e| e.to_string())?;
    assert_eq!(
        session
            .get_leaf_id("main")
            .map_err(|e| e.to_string())?
            .as_deref(),
        Some(main_child.as_str())
    );
    assert_eq!(
        session
            .get_leaf_id("thread")
            .map_err(|e| e.to_string())?
            .as_deref(),
        Some(thread_child.as_str())
    );
    assert_eq!(
        ids(&session
            .find_entries_on_branch(
                "main",
                EntryQuery {
                    order: Some("oldestFirst".into()),
                    ..EntryQuery::default()
                }
            )
            .map_err(|e| e.to_string())?),
        vec![root.clone(), main_child]
    );
    assert_eq!(
        ids(&session
            .find_entries_on_branch(
                "thread",
                EntryQuery {
                    order: Some("oldestFirst".into()),
                    ..EntryQuery::default()
                }
            )
            .map_err(|e| e.to_string())?),
        vec![root, thread_child]
    );
    let empty = repo
        .create(CreateOptions {
            id: Some("empty".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    assert!(empty
        .find_entries_on_branch("main", EntryQuery::default())
        .map_err(|e| e.to_string())?
        .is_empty());
    Ok(())
}

fn provisioned_ids<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let session = repo
        .create(CreateOptions {
            id: Some("session".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    let entry = session
        .append_entry(
            json!({"type":"custom","id":"provisioned","customType":"note","data":{"value":1}}),
            "main",
        )
        .map_err(|e| e.to_string())?;
    assert_eq!(entry.get("customType"), Some(&json!("note")));
    assert_eq!(entry.get("id"), Some(&json!("provisioned")));
    assert_eq!(entry.get("parentId"), Some(&Value::Null));
    assert_eq!(entry.get("seq"), Some(&json!(1)));
    assert_eq!(
        session
            .get_leaf_id("main")
            .map_err(|e| e.to_string())?
            .as_deref(),
        Some("provisioned")
    );
    Ok(())
}

fn tool_result_terminate<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let session = repo
        .create(CreateOptions {
            id: Some("session".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    let entry = session
        .append_entry(
            json!({
                "type":"message",
                "id":"tool-result",
                "message":{
                    "role":"toolResult","toolCallId":"call-1","toolName":"example",
                    "content":[{"type":"text","text":"done"}],"isError":false,"timestamp":1
                },
                "terminate": true
            }),
            "main",
        )
        .map_err(|e| e.to_string())?;
    assert_eq!(entry.get("terminate"), Some(&json!(true)));
    let stored = session
        .get_entry("tool-result")
        .map_err(|e| e.to_string())?
        .unwrap();
    assert_eq!(stored.get("terminate"), Some(&json!(true)));
    Ok(())
}

fn rejects_non_json_entries<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let session = repo
        .create(CreateOptions {
            id: Some("session".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    for reason in [
        "contains undefined",
        "contains a non-finite number",
        "contains bigint",
        "contains a non-plain object",
        "contains a cycle",
    ] {
        let err = BackendError::durable_payload(reason);
        assert_eq!(err.code, BackendErrorCode::InvalidPayload);
        assert_eq!(err.message, format!("Durable payload {reason}"));
    }
    assert_eq!(
        session.get_leaf_id("main").map_err(|e| e.to_string())?,
        None
    );
    assert!(session
        .find_entries(EntryQuery::default())
        .map_err(|e| e.to_string())?
        .is_empty());
    let valid = session
        .append_custom_entry("valid", Some(json!({"value":1})), "main")
        .map_err(|e| e.to_string())?;
    assert_eq!(
        session
            .get_entry(&valid)
            .map_err(|e| e.to_string())?
            .unwrap()
            .get("seq"),
        Some(&json!(1))
    );
    Ok(())
}

fn rejects_non_json_records<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let session = repo
        .create(CreateOptions {
            id: Some("session".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    for reason in ["contains undefined", "contains bigint"] {
        let err = BackendError::durable_payload(reason);
        assert_eq!(err.message, format!("Durable payload {reason}"));
    }
    assert!(session
        .find_records(RecordQuery::default())
        .map_err(|e| e.to_string())?
        .is_empty());
    assert_eq!(
        session
            .append_record(operation_started("valid-record", "main", "run"))
            .map_err(|e| e.to_string())?
            .get("seq"),
        Some(&json!(1))
    );
    Ok(())
}

fn concurrent_writes<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let session = repo
        .create(CreateOptions {
            id: Some("session".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    session
        .append_entry(
            json!({"type":"message","id":"root","message": user_message("root")}),
            "main",
        )
        .map_err(|e| e.to_string())?;
    session
        .create_lane("thread", Some("root"))
        .map_err(|e| e.to_string())?;
    let jobs = [
        ("main-1", "main"),
        ("thread-1", "thread"),
        ("main-2", "main"),
        ("thread-2", "thread"),
    ];
    let mut completion = Vec::new();
    let mut entries = Vec::new();
    for (id, lane) in jobs {
        let entry = session
            .append_entry(json!({"type":"custom","id": id, "customType":"note"}), lane)
            .map_err(|e| e.to_string())?;
        completion.push(id.to_string());
        entries.push(entry);
    }
    let mut commit: Vec<(u64, String)> = entries
        .iter()
        .map(|e| {
            (
                e.get("seq").and_then(Value::as_u64).unwrap_or(0),
                e.get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            )
        })
        .collect();
    commit.sort_by_key(|(seq, _)| *seq);
    let commit_ids: Vec<String> = commit.into_iter().map(|(_, id)| id).collect();
    assert_eq!(completion, commit_ids);
    let seqs = log_seqs(&session)?;
    let mut sorted = seqs.clone();
    sorted.sort();
    assert_eq!(seqs, sorted);
    Ok(())
}

fn create_list_open<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let session = repo
        .create(CreateOptions {
            id: Some("one".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    let entry_id = session
        .append_message(user_message("persisted"), "main")
        .map_err(|e| e.to_string())?;
    let metadata = session.get_metadata().map_err(|e| e.to_string())?;
    let listed = repo.list().map_err(|e| e.to_string())?;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, metadata.id);
    assert_eq!(listed[0].created_at, metadata.created_at);
    assert_eq!(listed[0].parent_session_id, metadata.parent_session_id);
    let opened = repo.open(&metadata).map_err(|e| e.to_string())?;
    assert_eq!(
        ids(&opened
            .find_entries(EntryQuery::default())
            .map_err(|e| e.to_string())?),
        vec![entry_id]
    );
    expect_code(
        repo.create(CreateOptions {
            id: Some("one".into()),
            ..CreateOptions::default()
        }),
        BackendErrorCode::AlreadyExists,
    )?;
    Ok(())
}

fn delete_idempotent<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let session = repo
        .create(CreateOptions {
            id: Some("one".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    let metadata = session.get_metadata().map_err(|e| e.to_string())?;
    repo.delete(&metadata).map_err(|e| e.to_string())?;
    expect_code(repo.open(&metadata), BackendErrorCode::NotFound)?;
    repo.delete(&metadata).map_err(|e| e.to_string())?;
    Ok(())
}

fn fork_branch<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let source = repo
        .create(CreateOptions {
            id: Some("source".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    let root = source
        .append_message(user_message("root"), "main")
        .map_err(|e| e.to_string())?;
    let shared = source
        .append_message(assistant_message("shared"), "main")
        .map_err(|e| e.to_string())?;
    source
        .create_lane("thread", Some(&shared))
        .map_err(|e| e.to_string())?;
    let thread_child = source
        .append_message(user_message("thread"), "thread")
        .map_err(|e| e.to_string())?;
    let main_child = source
        .append_message(user_message("main"), "main")
        .map_err(|e| e.to_string())?;
    source.set_name(Some("Source")).map_err(|e| e.to_string())?;
    source
        .set_label(&shared, Some("copied"))
        .map_err(|e| e.to_string())?;
    source
        .set_label(&thread_child, Some("excluded"))
        .map_err(|e| e.to_string())?;
    source
        .append_record(operation_started("run", "main", "run"))
        .map_err(|e| e.to_string())?;
    source
        .append_record(json!({
            "type":"usage","id":"source-usage","lane":"main","cause":"adjustment",
            "usage": {
                "input":10,"output":5,"cacheRead":3,"cacheWrite":2,"totalTokens":20,
                "cost":{"input":1,"output":2,"cacheRead":3,"cacheWrite":4,"total":10}
            }
        }))
        .map_err(|e| e.to_string())?;
    let fork = repo
        .fork(
            &source.get_metadata().map_err(|e| e.to_string())?,
            ForkOptions {
                scope: Some(ForkScope::Branch),
                entry_id: Some(main_child.clone()),
                position: Some(ForkPosition::At),
                id: Some("branch-fork".into()),
                ..ForkOptions::default()
            },
        )
        .map_err(|e| e.to_string())?;
    assert_eq!(
        ids(&fork
            .find_entries(EntryQuery {
                order: Some("oldestFirst".into()),
                ..EntryQuery::default()
            })
            .map_err(|e| e.to_string())?),
        vec![root, shared.clone(), main_child.clone()]
    );
    let lanes = fork.get_lanes().map_err(|e| e.to_string())?;
    assert_eq!(lanes.len(), 1);
    assert_eq!(lanes[0].leaf_id.as_deref(), Some(main_child.as_str()));
    assert_eq!(
        fork.get_name().map_err(|e| e.to_string())?.as_deref(),
        Some("Source")
    );
    assert_eq!(
        fork.get_label(&shared)
            .map_err(|e| e.to_string())?
            .as_deref(),
        Some("copied")
    );
    assert_eq!(
        fork.get_label(&thread_child).map_err(|e| e.to_string())?,
        None
    );
    assert!(fork
        .find_records(RecordQuery::default())
        .map_err(|e| e.to_string())?
        .is_empty());
    let stats = fork.get_stats().map_err(|e| e.to_string())?;
    assert_eq!(stats.message_count, 3);
    assert_eq!(stats.total_tokens, 0.0);
    fork.append_message(user_message("after fork"), "main")
        .map_err(|e| e.to_string())?;
    assert_eq!(
        fork.get_stats().map_err(|e| e.to_string())?.message_count,
        4
    );
    let metadata = fork.get_metadata().map_err(|e| e.to_string())?;
    assert_eq!(metadata.id, "branch-fork");
    assert_eq!(metadata.parent_session_id.as_deref(), Some("source"));
    Ok(())
}

fn fork_tree<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let source = repo
        .create(CreateOptions {
            id: Some("source".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    let root = source
        .append_message(user_message("root"), "main")
        .map_err(|e| e.to_string())?;
    source
        .create_lane("thread", Some(&root))
        .map_err(|e| e.to_string())?;
    let main_child = source
        .append_message(user_message("main"), "main")
        .map_err(|e| e.to_string())?;
    let thread_child = source
        .append_message(user_message("thread"), "thread")
        .map_err(|e| e.to_string())?;
    source
        .set_label(&thread_child, Some("thread-tip"))
        .map_err(|e| e.to_string())?;
    let fork = repo
        .fork(
            &source.get_metadata().map_err(|e| e.to_string())?,
            ForkOptions {
                scope: Some(ForkScope::Tree),
                id: Some("tree-fork".into()),
                ..ForkOptions::default()
            },
        )
        .map_err(|e| e.to_string())?;
    assert_eq!(
        ids(&fork
            .find_entries(EntryQuery {
                order: Some("oldestFirst".into()),
                ..EntryQuery::default()
            })
            .map_err(|e| e.to_string())?),
        vec![root, main_child.clone(), thread_child.clone()]
    );
    let lanes = fork.get_lanes().map_err(|e| e.to_string())?;
    assert_eq!(lanes[0].leaf_id.as_deref(), Some(main_child.as_str()));
    assert_eq!(lanes[1].leaf_id.as_deref(), Some(thread_child.as_str()));
    assert_eq!(
        fork.get_label(&thread_child)
            .map_err(|e| e.to_string())?
            .as_deref(),
        Some("thread-tip")
    );
    assert_eq!(
        fork.get_stats().map_err(|e| e.to_string())?.message_count,
        3
    );
    let lanes_in_log: Vec<_> = fork
        .get_log(LogOptions::default())
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter_map(|item| match item {
            LogItem::Lane { seq, lane, leaf_id } => Some((seq, lane, leaf_id)),
            _ => None,
        })
        .collect();
    assert_eq!(lanes_in_log[0], (4, "main".into(), Some(main_child)));
    assert_eq!(lanes_in_log[1], (5, "thread".into(), Some(thread_child)));
    Ok(())
}

fn fork_before<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let source = repo
        .create(CreateOptions {
            id: Some("source".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    let root = source
        .append_message(user_message("root"), "main")
        .map_err(|e| e.to_string())?;
    let tail = source
        .append_message(user_message("tail"), "main")
        .map_err(|e| e.to_string())?;
    let metadata = source.get_metadata().map_err(|e| e.to_string())?;
    let fork = repo
        .fork(
            &metadata,
            ForkOptions {
                entry_id: Some(tail.clone()),
                id: Some("fork".into()),
                ..ForkOptions::default()
            },
        )
        .map_err(|e| e.to_string())?;
    assert_eq!(
        ids(&fork
            .find_entries(EntryQuery {
                order: Some("oldestFirst".into()),
                ..EntryQuery::default()
            })
            .map_err(|e| e.to_string())?),
        vec![root.clone()]
    );
    assert_eq!(
        fork.get_leaf_id("main")
            .map_err(|e| e.to_string())?
            .as_deref(),
        Some(root.as_str())
    );
    assert_eq!(
        source
            .get_leaf_id("main")
            .map_err(|e| e.to_string())?
            .as_deref(),
        Some(tail.as_str())
    );
    let before_default = repo
        .fork(
            &metadata,
            ForkOptions {
                position: Some(ForkPosition::Before),
                id: Some("before-default-target".into()),
                ..ForkOptions::default()
            },
        )
        .map_err(|e| e.to_string())?;
    assert_eq!(
        ids(&before_default
            .find_entries(EntryQuery {
                order: Some("oldestFirst".into()),
                ..EntryQuery::default()
            })
            .map_err(|e| e.to_string())?),
        vec![root.clone()]
    );
    let at_default = repo
        .fork(
            &metadata,
            ForkOptions {
                position: Some(ForkPosition::At),
                id: Some("at-default-target".into()),
                ..ForkOptions::default()
            },
        )
        .map_err(|e| e.to_string())?;
    assert_eq!(
        ids(&at_default
            .find_entries(EntryQuery {
                order: Some("oldestFirst".into()),
                ..EntryQuery::default()
            })
            .map_err(|e| e.to_string())?),
        vec![root, tail]
    );
    expect_code(
        repo.fork(
            &metadata,
            ForkOptions {
                entry_id: Some("missing".into()),
                ..ForkOptions::default()
            },
        ),
        BackendErrorCode::InvalidForkTarget,
    )?;
    Ok(())
}

fn fork_default_target<R: SessionRepository>(repo: &R) -> Result<(), String> {
    let source = repo
        .create(CreateOptions {
            id: Some("source-with-custom-leaf".into()),
            ..CreateOptions::default()
        })
        .map_err(|e| e.to_string())?;
    source
        .append_custom_entry("not-a-message", None, "main")
        .map_err(|e| e.to_string())?;
    expect_code(
        repo.fork(
            &source.get_metadata().map_err(|e| e.to_string())?,
            ForkOptions {
                id: Some("fork".into()),
                ..ForkOptions::default()
            },
        ),
        BackendErrorCode::InvalidForkTarget,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MemorySessionRepo;

    #[test]
    fn memory_matches_ts_conformance() {
        run_all(MemorySessionRepo::new).expect("memory conformance");
    }
}
