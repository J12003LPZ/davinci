use pi_core::{SessionError, WriterLease};
use rusqlite::{params, Connection, OptionalExtension};

pub fn acquire_writer_lease(
    db: &Connection,
    session_id: &str,
    owner_id: &str,
    now: i64,
    expires_at_ms: i64,
) -> Result<Option<WriterLease>, rusqlite::Error> {
    let row = db
        .query_row(
            "INSERT INTO writer_leases (session_id, owner_id, fence, expires_at_ms)
             VALUES (?1, ?2, 1, ?3)
             ON CONFLICT(session_id) DO UPDATE SET
                owner_id = excluded.owner_id,
                fence = writer_leases.fence + 1,
                expires_at_ms = excluded.expires_at_ms
             WHERE writer_leases.expires_at_ms <= ?4
             RETURNING owner_id, fence, expires_at_ms",
            params![session_id, owner_id, expires_at_ms, now],
            |row| {
                Ok(WriterLease {
                    owner_id: row.get(0)?,
                    fence: row.get(1)?,
                    expires_at_ms: row.get(2)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

pub fn renew_writer_lease(
    db: &Connection,
    session_id: &str,
    lease: &mut WriterLease,
    now: i64,
    expires_at_ms: i64,
) -> Result<bool, rusqlite::Error> {
    let changes = db.execute(
        "UPDATE writer_leases
         SET expires_at_ms = ?1
         WHERE session_id = ?2
           AND owner_id = ?3
           AND fence = ?4
           AND expires_at_ms > ?5",
        params![expires_at_ms, session_id, lease.owner_id, lease.fence, now],
    )?;
    if changes == 1 {
        lease.expires_at_ms = expires_at_ms;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn release_writer_lease(
    db: &Connection,
    session_id: &str,
    lease: &WriterLease,
) -> Result<(), rusqlite::Error> {
    db.execute(
        "DELETE FROM writer_leases
         WHERE session_id = ?1 AND owner_id = ?2 AND fence = ?3",
        params![session_id, lease.owner_id, lease.fence],
    )?;
    Ok(())
}

pub fn delete_writer_lease(db: &Connection, session_id: &str) -> Result<(), rusqlite::Error> {
    db.execute(
        "DELETE FROM writer_leases WHERE session_id = ?1",
        params![session_id],
    )?;
    Ok(())
}

pub fn read_writer_lease(
    db: &Connection,
    session_id: &str,
) -> Result<Option<WriterLease>, rusqlite::Error> {
    db.query_row(
        "SELECT owner_id, fence, expires_at_ms FROM writer_leases WHERE session_id = ?1",
        params![session_id],
        |row| {
            Ok(WriterLease {
                owner_id: row.get(0)?,
                fence: row.get(1)?,
                expires_at_ms: row.get(2)?,
            })
        },
    )
    .optional()
}

pub fn claim_writer_lease(
    db: &Connection,
    session_id: &str,
    ttl_ms: i64,
    now: i64,
) -> Result<WriterLease, SessionError> {
    let owner_id = uuid::Uuid::now_v7().to_string();
    acquire_writer_lease(db, session_id, &owner_id, now, now + ttl_ms)
        .map_err(|e| SessionError::new(pi_core::SessionErrorCode::Storage, e.to_string()))?
        .ok_or_else(|| SessionError::active_writer(session_id))
}
