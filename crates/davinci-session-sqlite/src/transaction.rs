//! Atomic write scopes that preserve the store's shared-reference API.

use davinci_session::SessionError;
use rusqlite::Connection;

pub(crate) fn atomic<T>(
    conn: &Connection,
    write: impl FnOnce() -> Result<T, SessionError>,
) -> Result<T, SessionError> {
    let outermost = conn.is_autocommit();
    conn.execute_batch(if outermost {
        "BEGIN IMMEDIATE"
    } else {
        "SAVEPOINT davinci_write"
    })
    .map_err(|err| SessionError::storage(format!("Unable to begin atomic write: {err}")))?;
    let mut guard = WriteGuard {
        conn,
        outermost,
        committed: false,
    };
    let result = write()?;
    conn.execute_batch(if outermost {
        "COMMIT"
    } else {
        "RELEASE SAVEPOINT davinci_write"
    })
    .map_err(|err| SessionError::storage(format!("Unable to commit atomic write: {err}")))?;
    guard.committed = true;
    Ok(result)
}

struct WriteGuard<'a> {
    conn: &'a Connection,
    outermost: bool,
    committed: bool,
}

impl Drop for WriteGuard<'_> {
    fn drop(&mut self) {
        if !self.committed {
            // Also roll back on unwinding or a failed COMMIT. Nested savepoints
            // use SQLite's innermost-name resolution and never roll back a caller's transaction.
            let _ = self.conn.execute_batch(if self.outermost {
                "ROLLBACK"
            } else {
                "ROLLBACK TO SAVEPOINT davinci_write; RELEASE SAVEPOINT davinci_write"
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_error_and_panic_preserve_callers_transaction() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE items (value INTEGER); BEGIN; INSERT INTO items VALUES (1)",
        )
        .unwrap();
        let result: Result<(), SessionError> = atomic(&conn, || {
            conn.execute("INSERT INTO items VALUES (2)", []).unwrap();
            Err(SessionError::storage("injected"))
        });
        assert!(result.is_err());
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _: Result<(), SessionError> = atomic(&conn, || {
                conn.execute("INSERT INTO items VALUES (3)", []).unwrap();
                panic!("injected panic");
            });
        }));
        assert!(panic.is_err());
        assert!(!conn.is_autocommit());
        conn.execute_batch("COMMIT").unwrap();
        assert_eq!(
            conn.query_row("SELECT sum(value) FROM items", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}
