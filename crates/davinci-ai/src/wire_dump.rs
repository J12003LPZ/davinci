//! Opt-in request and usage dumps for prompt-cache diagnosis.
//! No TypeScript counterpart. DAVINCI_WIRE_DUMP files contain conversation
//! content, including files the agent read. Keep the directory local.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
thread_local! {
    static CURRENT: RefCell<Option<(u64, PathBuf)>> = const { RefCell::new(None) };
}

/// A synchronous request's dump scope. Concurrent threads and nested calls
/// have independent records. Dropping the scope restores the previous record.
pub struct RequestDump {
    sequence: u64,
    directory: PathBuf,
    previous: Option<(u64, PathBuf)>,
    _thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl RequestDump {
    fn start(directory: PathBuf) -> Self {
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1;
        let previous = CURRENT.with(|current| current.replace(Some((sequence, directory.clone()))));
        Self {
            sequence,
            directory,
            previous,
            _thread_bound: std::marker::PhantomData,
        }
    }

    pub fn write(&self, kind: &str, value: &serde_json::Value) {
        write_to(&self.directory, self.sequence, kind, value);
    }
}

impl Drop for RequestDump {
    fn drop(&mut self) {
        CURRENT.with(|current| current.replace(self.previous.take()));
    }
}

/// Begin a request record only when the user explicitly enables dumping.
pub fn begin() -> Option<RequestDump> {
    std::env::var_os("DAVINCI_WIRE_DUMP")
        .filter(|value| !value.is_empty())
        .map(|directory| RequestDump::start(PathBuf::from(directory)))
}

/// Record a frame for the active synchronous request on this thread.
pub fn write_current(kind: &str, value: &serde_json::Value) {
    CURRENT.with(|current| {
        if let Some((seq, dir)) = current.borrow().as_ref() {
            write_to(dir, *seq, kind, value);
        }
    });
}

/// Dump failures must never prevent a provider request from completing.
pub fn write_to(dir: &Path, seq: u64, kind: &str, value: &serde_json::Value) {
    if !matches!(kind, "logical" | "wire" | "usage") || std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let path = dir.join(format!("{seq:04}-{}-{kind}.json", std::process::id()));
    if let Ok(text) = serde_json::to_string_pretty(value) {
        let _ = std::fs::write(path, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_one_numbered_file_per_kind() {
        let dir = tempfile::tempdir().unwrap();
        write_to(
            dir.path(),
            7,
            "logical",
            &serde_json::json!({"input": [1, 2]}),
        );
        write_to(
            dir.path(),
            7,
            "usage",
            &serde_json::json!({"input": 5, "cacheRead": 3}),
        );
        let name = |kind: &str| format!("0007-{}-{kind}.json", std::process::id());
        let logical: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join(name("logical"))).unwrap(),
        )
        .unwrap();
        assert_eq!(logical["input"][1], 2);
        assert!(dir.path().join(name("usage")).exists());
    }

    #[test]
    fn concurrent_requests_keep_wire_and_usage_in_their_own_record() {
        let dir = tempfile::tempdir().unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|id| {
                let dir = dir.path().to_path_buf();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let dump = RequestDump::start(dir);
                    barrier.wait();
                    write_current("wire", &serde_json::json!({"id": id}));
                    dump.write("usage", &serde_json::json!({"id": id}));
                    dump.sequence
                })
            })
            .collect();
        for handle in handles {
            let seq = handle.join().unwrap();
            let read = |kind| {
                std::fs::read_to_string(
                    dir.path()
                        .join(format!("{seq:04}-{}-{kind}.json", std::process::id())),
                )
                .unwrap()
            };
            assert_eq!(read("wire"), read("usage"));
        }
    }

    #[test]
    fn nested_requests_restore_the_outer_record_and_clear_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let outer = RequestDump::start(dir.path().to_path_buf());
        {
            let _inner = RequestDump::start(dir.path().to_path_buf());
            write_current("wire", &serde_json::json!("inner"));
        }
        write_current("wire", &serde_json::json!("outer"));
        let path = dir.path().join(format!(
            "{:04}-{}-wire.json",
            outer.sequence,
            std::process::id()
        ));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "\"outer\"");
        drop(outer);
        assert!(CURRENT.with(|current| current.borrow().is_none()));
    }
}
