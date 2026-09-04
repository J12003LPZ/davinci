//! Derived branch cache matching `sqlite-node/src/sqlite/branch-cache.ts`.

use davinci_session::SessionError;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use uuid::Uuid;

pub fn delete_branch_cache(conn: &Connection, session_id: &str) -> Result<(), SessionError> {
    conn.execute(
        "DELETE FROM branch_tips WHERE session_id = ?1",
        [session_id],
    )
    .map_err(|err| SessionError::storage(format!("Unable to delete branch tips: {err}")))?;
    conn.execute(
        "DELETE FROM branch_entries WHERE session_id = ?1",
        [session_id],
    )
    .map_err(|err| SessionError::storage(format!("Unable to delete branch entries: {err}")))?;
    Ok(())
}

pub fn rebuild_branch_cache(conn: &Connection, session_id: &str) -> Result<(), SessionError> {
    let mut stmt = conn
        .prepare(
            "SELECT leaf.id
             FROM entries AS leaf
             WHERE leaf.session_id = ?1
               AND NOT EXISTS (
                   SELECT 1 FROM entries AS child
                   WHERE child.session_id = leaf.session_id AND child.parent_id = leaf.id
               )
             ORDER BY leaf.seq",
        )
        .map_err(|err| SessionError::storage(format!("Unable to list branch tips: {err}")))?;
    let tips = stmt
        .query_map([session_id], |row| row.get::<_, String>(0))
        .map_err(|err| SessionError::storage(format!("Unable to query branch tips: {err}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| SessionError::storage(format!("Unable to read branch tips: {err}")))?;
    drop(stmt);
    delete_branch_cache(conn, session_id)?;
    for tip in tips {
        build_cached_branch(conn, session_id, &tip)?;
    }
    Ok(())
}

pub fn build_cached_branch(
    conn: &Connection,
    session_id: &str,
    leaf_id: &str,
) -> Result<(), SessionError> {
    conn.execute("SAVEPOINT build_branch_cache", [])
        .map_err(|err| SessionError::storage(format!("Unable to open branch savepoint: {err}")))?;
    let result: Result<(), SessionError> = (|| {
        let branch_id = Uuid::now_v7().to_string();
        insert_branch_entries_for_path(conn, session_id, &branch_id, leaf_id)?;
        insert_branch_tip(conn, session_id, leaf_id, &branch_id)?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            conn.execute("RELEASE SAVEPOINT build_branch_cache", [])
                .map_err(|err| {
                    SessionError::storage(format!("Unable to release branch savepoint: {err}"))
                })?;
            Ok(())
        }
        Err(error) => {
            let _ = conn.execute("ROLLBACK TO SAVEPOINT build_branch_cache", []);
            let _ = conn.execute("RELEASE SAVEPOINT build_branch_cache", []);
            if error.code == "invalid_entry" {
                return Err(error);
            }
            Err(SessionError::storage(format!(
                "Failed to build SQLite branch cache at entry {leaf_id}"
            )))
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn append_entry_to_branch_cache(
    conn: &Connection,
    session_id: &str,
    entry_id: &str,
    entry_seq: i64,
    entry_type: &str,
    custom_type: Option<&str>,
    parent_id: Option<&str>,
) -> Result<(), SessionError> {
    let Some(parent_id) = parent_id else {
        let branch_id = Uuid::now_v7().to_string();
        insert_branch_entry(
            conn,
            session_id,
            &branch_id,
            entry_id,
            entry_seq,
            entry_type,
            custom_type,
        )?;
        insert_branch_tip(conn, session_id, entry_id, &branch_id)?;
        return Ok(());
    };

    if let Some(tip_branch_id) = read_branch_tip_branch_id(conn, session_id, parent_id)? {
        extend_branch(
            conn,
            session_id,
            &tip_branch_id,
            parent_id,
            entry_id,
            entry_seq,
            entry_type,
            custom_type,
        )?;
        return Ok(());
    }

    let Some(source) = read_branch_containing_entry(conn, session_id, parent_id)? else {
        return Err(SessionError::invalid_entry(format!(
            "Branch cache has no branch containing parent entry {parent_id}"
        )));
    };

    let branch_id = Uuid::now_v7().to_string();
    copy_branch_entries_through_seq(conn, session_id, &branch_id, &source.0, source.1)?;
    insert_branch_entry(
        conn,
        session_id,
        &branch_id,
        entry_id,
        entry_seq,
        entry_type,
        custom_type,
    )?;
    insert_branch_tip(conn, session_id, entry_id, &branch_id)?;
    Ok(())
}

pub fn read_cached_branch_ids(
    conn: &Connection,
    session_id: &str,
    branch_id: &str,
) -> Result<Vec<String>, SessionError> {
    let mut stmt = conn
        .prepare(
            "SELECT entry_id FROM branch_entries
             WHERE session_id = ?1 AND branch_id = ?2
             ORDER BY entry_seq",
        )
        .map_err(|err| SessionError::storage(format!("Unable to query cached branch: {err}")))?;
    let rows = stmt
        .query_map(params![session_id, branch_id], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|err| SessionError::storage(format!("Unable to read cached branch: {err}")))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|err| SessionError::storage(format!("Unable to collect cached branch: {err}")))
}

pub fn branch_id_for_entry(
    conn: &Connection,
    session_id: &str,
    entry_id: &str,
) -> Result<Option<String>, SessionError> {
    conn.query_row(
        "SELECT branch_id FROM branch_entries
         WHERE session_id = ?1 AND entry_id = ?2
         ORDER BY branch_id
         LIMIT 1",
        params![session_id, entry_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|err| SessionError::storage(format!("Unable to read cached branch id: {err}")))
}

#[allow(clippy::too_many_arguments)]
fn extend_branch(
    conn: &Connection,
    session_id: &str,
    branch_id: &str,
    parent_id: &str,
    entry_id: &str,
    entry_seq: i64,
    entry_type: &str,
    custom_type: Option<&str>,
) -> Result<(), SessionError> {
    insert_branch_entry(
        conn,
        session_id,
        branch_id,
        entry_id,
        entry_seq,
        entry_type,
        custom_type,
    )?;
    if !update_branch_tip(conn, session_id, branch_id, parent_id, entry_id)? {
        return Err(SessionError::invalid_entry(format!(
            "Branch tip {parent_id} changed during append"
        )));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn insert_branch_entry(
    conn: &Connection,
    session_id: &str,
    branch_id: &str,
    entry_id: &str,
    entry_seq: i64,
    entry_type: &str,
    custom_type: Option<&str>,
) -> Result<(), SessionError> {
    conn.execute(
        "INSERT INTO branch_entries
            (session_id, branch_id, entry_id, entry_seq, entry_type, custom_type)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            session_id,
            branch_id,
            entry_id,
            entry_seq,
            entry_type,
            custom_type
        ],
    )
    .map_err(|err| SessionError::storage(format!("Unable to insert branch entry: {err}")))?;
    Ok(())
}

fn insert_branch_tip(
    conn: &Connection,
    session_id: &str,
    tip_id: &str,
    branch_id: &str,
) -> Result<(), SessionError> {
    conn.execute(
        "INSERT INTO branch_tips (session_id, tip_id, branch_id) VALUES (?1, ?2, ?3)",
        params![session_id, tip_id, branch_id],
    )
    .map_err(|err| SessionError::storage(format!("Unable to insert branch tip: {err}")))?;
    Ok(())
}

fn update_branch_tip(
    conn: &Connection,
    session_id: &str,
    branch_id: &str,
    old_tip_id: &str,
    new_tip_id: &str,
) -> Result<bool, SessionError> {
    let changed = conn
        .execute(
            "UPDATE branch_tips SET tip_id = ?1
             WHERE session_id = ?2 AND branch_id = ?3 AND tip_id = ?4",
            params![new_tip_id, session_id, branch_id, old_tip_id],
        )
        .map_err(|err| SessionError::storage(format!("Unable to update branch tip: {err}")))?;
    Ok(changed == 1)
}

fn read_branch_tip_branch_id(
    conn: &Connection,
    session_id: &str,
    tip_id: &str,
) -> Result<Option<String>, SessionError> {
    conn.query_row(
        "SELECT branch_id FROM branch_tips WHERE session_id = ?1 AND tip_id = ?2",
        params![session_id, tip_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|err| SessionError::storage(format!("Unable to read branch tip: {err}")))
}

fn read_branch_containing_entry(
    conn: &Connection,
    session_id: &str,
    entry_id: &str,
) -> Result<Option<(String, i64)>, SessionError> {
    conn.query_row(
        "SELECT branch_id, entry_seq FROM branch_entries
         WHERE session_id = ?1 AND entry_id = ?2
         ORDER BY branch_id
         LIMIT 1",
        params![session_id, entry_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(|err| SessionError::storage(format!("Unable to read cached parent: {err}")))
}

fn copy_branch_entries_through_seq(
    conn: &Connection,
    session_id: &str,
    target_branch_id: &str,
    source_branch_id: &str,
    through_seq: i64,
) -> Result<(), SessionError> {
    conn.execute(
        "INSERT INTO branch_entries (session_id, branch_id, entry_id, entry_seq, entry_type, custom_type)
         SELECT session_id, ?1, entry_id, entry_seq, entry_type, custom_type
         FROM branch_entries
         WHERE session_id = ?2 AND branch_id = ?3 AND entry_seq <= ?4",
        params![target_branch_id, session_id, source_branch_id, through_seq],
    )
    .map_err(|err| SessionError::storage(format!("Unable to copy cached branch: {err}")))?;
    Ok(())
}

fn insert_branch_entries_for_path(
    conn: &Connection,
    session_id: &str,
    branch_id: &str,
    leaf_id: &str,
) -> Result<(), SessionError> {
    let mut path = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut entry_id = Some(leaf_id.to_string());
    while let Some(current) = entry_id {
        if !seen.insert(current.clone()) {
            return Err(SessionError::invalid_entry(format!(
                "Entry parent cycle at {current}"
            )));
        }
        let row: Option<(String, i64, Option<String>, String, String)> = conn
            .query_row(
                "SELECT id, seq, parent_id, type, payload
                 FROM entries
                 WHERE session_id = ?1 AND id = ?2",
                params![session_id, current],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|err| SessionError::storage(format!("Unable to walk branch path: {err}")))?;
        let Some((id, seq, parent_id, entry_type, payload)) = row else {
            return Err(SessionError::invalid_entry(format!(
                "Entry {current} not found"
            )));
        };
        let custom_type = custom_type_from_payload(&id, &entry_type, &payload)?;
        path.push((id, seq, entry_type, custom_type));
        entry_id = parent_id;
    }
    for (id, seq, entry_type, custom_type) in path.into_iter().rev() {
        insert_branch_entry(
            conn,
            session_id,
            branch_id,
            &id,
            seq,
            &entry_type,
            custom_type.as_deref(),
        )?;
    }
    Ok(())
}

fn custom_type_from_payload(
    id: &str,
    entry_type: &str,
    payload: &str,
) -> Result<Option<String>, SessionError> {
    if entry_type != "custom" {
        return Ok(None);
    }
    let parsed: Value = serde_json::from_str(payload).map_err(|_| {
        SessionError::invalid_entry(format!(
            "Invalid SQLite session entry {id}: failed to decode entry {id}"
        ))
    })?;
    if !parsed.is_object() {
        return Err(SessionError::invalid_entry(format!(
            "Invalid SQLite session entry {id}: failed to decode entry {id}"
        )));
    }
    match parsed.get("customType").and_then(Value::as_str) {
        Some(custom) => Ok(Some(custom.to_string())),
        None => Err(SessionError::invalid_entry(format!(
            "Invalid SQLite session entry {id}: failed to decode entry {id}"
        ))),
    }
}
