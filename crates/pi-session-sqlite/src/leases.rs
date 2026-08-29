use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use pi_core::{now_ms, SessionError};
use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriterLease {
    pub owner_id: String,
    pub fence: i64,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, Copy)]
pub struct WriterLeaseOptions {
    pub ttl_ms: i64,
    pub heartbeat_interval_ms: i64,
}

impl Default for WriterLeaseOptions {
    fn default() -> Self {
        Self {
            ttl_ms: 30_000,
            heartbeat_interval_ms: 10_000,
        }
    }
}

impl WriterLeaseOptions {
    pub fn resolve(
        ttl_ms: Option<i64>,
        heartbeat_interval_ms: Option<i64>,
    ) -> Result<Self, SessionError> {
        let ttl_ms = ttl_ms.unwrap_or(30_000);
        let heartbeat_interval_ms = heartbeat_interval_ms.unwrap_or(10_000);
        if ttl_ms <= 0 {
            return Err(SessionError::invalid_payload(
                "writerLease.ttlMs must be positive",
            ));
        }
        if heartbeat_interval_ms <= 0 || heartbeat_interval_ms >= ttl_ms {
            return Err(SessionError::invalid_payload(
                "writerLease.heartbeatIntervalMs must be positive and less than ttlMs",
            ));
        }
        Ok(Self {
            ttl_ms,
            heartbeat_interval_ms,
        })
    }
}

pub fn active_writer_error(session_id: &str) -> SessionError {
    SessionError::storage(format!(
        "SQLite session {session_id} already has an active writer"
    ))
}

pub fn lost_writer_error(session_id: &str) -> SessionError {
    SessionError::storage(format!("SQLite session {session_id} writer lease was lost"))
}

pub fn acquire_writer_lease(
    db: &Connection,
    session_id: &str,
    owner_id: &str,
    now: i64,
    expires_at_ms: i64,
) -> Result<Option<WriterLease>, SessionError> {
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
        .optional()
        .map_err(|error| SessionError::storage(error.to_string()))?;
    Ok(row)
}

pub fn renew_writer_lease(
    db: &Connection,
    session_id: &str,
    lease: &mut WriterLease,
    now: i64,
    expires_at_ms: i64,
) -> Result<bool, SessionError> {
    let changes = db
        .execute(
            "UPDATE writer_leases
		SET expires_at_ms = ?1
		WHERE session_id = ?2
			AND owner_id = ?3
			AND fence = ?4
			AND expires_at_ms > ?5",
            params![expires_at_ms, session_id, lease.owner_id, lease.fence, now],
        )
        .map_err(|error| SessionError::storage(error.to_string()))?;
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
) -> Result<(), SessionError> {
    db.execute(
        "DELETE FROM writer_leases
		WHERE session_id = ?1 AND owner_id = ?2 AND fence = ?3",
        params![session_id, lease.owner_id, lease.fence],
    )
    .map_err(|error| SessionError::storage(error.to_string()))?;
    Ok(())
}

pub fn delete_writer_lease(db: &Connection, session_id: &str) -> Result<(), SessionError> {
    db.execute(
        "DELETE FROM writer_leases WHERE session_id = ?1",
        params![session_id],
    )
    .map_err(|error| SessionError::storage(error.to_string()))?;
    Ok(())
}

/// Background renewer that extends an owned writer lease every `heartbeat_interval_ms`.
pub struct LeaseHeartbeat {
    stop: Arc<(Mutex<bool>, Condvar)>,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl LeaseHeartbeat {
    pub fn spawn(
        db: Arc<Mutex<Connection>>,
        session_id: String,
        lease: Arc<Mutex<WriterLease>>,
        options: WriterLeaseOptions,
    ) -> Self {
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let stop_for_thread = Arc::clone(&stop);
        let interval = Duration::from_millis(options.heartbeat_interval_ms.max(1) as u64);
        let handle = thread::spawn(move || {
            heartbeat_loop(stop_for_thread, db, session_id, lease, options, interval);
        });
        Self {
            stop,
            join: Mutex::new(Some(handle)),
        }
    }

    pub fn stop(&self) {
        if let Ok(mut stopped) = self.stop.0.lock() {
            *stopped = true;
        }
        self.stop.1.notify_all();
        if let Ok(mut join) = self.join.lock() {
            if let Some(handle) = join.take() {
                let _ = handle.join();
            }
        }
    }
}

impl Drop for LeaseHeartbeat {
    fn drop(&mut self) {
        self.stop();
    }
}

fn heartbeat_loop(
    stop: Arc<(Mutex<bool>, Condvar)>,
    db: Arc<Mutex<Connection>>,
    session_id: String,
    lease: Arc<Mutex<WriterLease>>,
    options: WriterLeaseOptions,
    interval: Duration,
) {
    loop {
        let (lock, cv) = &*stop;
        let Ok(stopped) = lock.lock() else {
            break;
        };
        if *stopped {
            break;
        }
        let Ok((guard, wait)) = cv.wait_timeout(stopped, interval) else {
            break;
        };
        if *guard {
            break;
        }
        drop(guard);
        if !wait.timed_out() {
            continue;
        }
        let now = now_ms();
        let Ok(db) = db.lock() else {
            break;
        };
        let Ok(mut lease) = lease.lock() else {
            break;
        };
        if !renew_writer_lease(&db, &session_id, &mut lease, now, now + options.ttl_ms)
            .unwrap_or(false)
        {
            break;
        }
    }
}

pub fn read_writer_leases(
    db: &Connection,
) -> Result<Vec<(String, String, i64, i64)>, SessionError> {
    let mut stmt = db
        .prepare(
            "SELECT session_id, owner_id, fence, expires_at_ms FROM writer_leases ORDER BY session_id",
        )
        .map_err(|error| SessionError::storage(error.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(|error| SessionError::storage(error.to_string()))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| SessionError::storage(error.to_string()))
}
