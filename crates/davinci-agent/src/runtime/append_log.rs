//! All-or-nothing appends to the runtime's local line ledgers (the worker
//! control receipt ledger and workflow artifact manifests).

use std::fs::OpenOptions;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Mutex;

/// One appender at a time in this process, so a rollback can never cut off
/// another thread's record.
static APPEND: Mutex<()> = Mutex::new(());

/// Append `record` and a newline, then fsync. Either the whole line is
/// durable or the file is cut back to its previous length: a failed write
/// or sync never leaves a partial or unsynced record for a later append to
/// fuse with or a restart to replay. A torn tail left by an earlier crash
/// gets a newline first, so this record starts on its own line.
pub(crate) fn append_record(path: &Path, record: &[u8]) -> io::Result<()> {
    let _guard = APPEND
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)?;
    let len = file.metadata()?.len();
    let mut line = Vec::with_capacity(record.len() + 2);
    if len > 0 {
        let mut last = [0u8; 1];
        file.seek(SeekFrom::End(-1))?;
        file.read_exact(&mut last)?;
        if last[0] != b'\n' {
            line.push(b'\n');
        }
    }
    line.extend_from_slice(record);
    line.push(b'\n');
    let written = file.write_all(&line).and_then(|()| {
        #[cfg(test)]
        if tests::FAIL_SYNC.with(|fail| fail.get()) {
            return Err(io::Error::other("injected sync failure"));
        }
        file.sync_data()
    });
    if written.is_err() {
        // Append mode cannot truncate on every platform; a plain write
        // handle can.
        if let Ok(truncate) = OpenOptions::new().write(true).open(path) {
            let _ = truncate.set_len(len);
            let _ = truncate.sync_data();
        }
    }
    written
}

#[cfg(test)]
pub(crate) mod tests {
    use super::append_record;

    thread_local! {
        /// Makes the next appends on this thread fail after writing, before
        /// the sync, the worst point for a partial record.
        pub(crate) static FAIL_SYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    #[test]
    fn a_failed_append_leaves_the_file_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.jsonl");
        append_record(&path, b"{\"a\":1}").unwrap();
        FAIL_SYNC.with(|fail| fail.set(true));
        assert!(append_record(&path, b"{\"b\":2}").is_err());
        FAIL_SYNC.with(|fail| fail.set(false));
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"a\":1}\n");
        append_record(&path, b"{\"c\":3}").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"a\":1}\n{\"c\":3}\n");
    }

    #[test]
    fn an_append_after_a_torn_tail_starts_a_fresh_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.jsonl");
        std::fs::write(&path, b"{\"a\":1}\n{\"tor").unwrap();
        append_record(&path, b"{\"b\":2}").unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"{\"a\":1}\n{\"tor\n{\"b\":2}\n"
        );
    }
}
