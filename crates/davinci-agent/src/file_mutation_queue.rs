//! Serialize write/edit mutations per realpath, matching
//! `vendor/pi/packages/coding-agent/src/core/tools/file-mutation-queue.ts`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

static QUEUES: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();

fn queues() -> &'static Mutex<HashMap<String, Arc<Mutex<()>>>> {
    QUEUES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// TS `getMutationQueueKey`: `realpath` when the path exists, otherwise `resolve`.
pub fn mutation_queue_key(file_path: &Path) -> String {
    let resolved = if file_path.is_absolute() {
        file_path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(file_path)
    };
    match resolved.canonicalize() {
        Ok(real) => real.to_string_lossy().into_owned(),
        Err(_) => resolved.to_string_lossy().into_owned(),
    }
}

/// Hold the per-file lock for the duration of `fn`.
pub fn with_file_mutation_queue<T>(file_path: &Path, func: impl FnOnce() -> T) -> T {
    let key = mutation_queue_key(file_path);
    let lock = {
        let mut map = queues().lock().unwrap_or_else(|err| err.into_inner());
        map.entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let _guard = lock.lock().unwrap_or_else(|err| err.into_inner());
    func()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn serializes_operations_for_the_same_file() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let dir = tempdir().unwrap();
        let path = dir.path().join("same.txt");
        let first_order = order.clone();
        let first_path = path.clone();
        let first = thread::spawn(move || {
            with_file_mutation_queue(&first_path, || {
                first_order.lock().unwrap().push("first:start");
                thread::sleep(Duration::from_millis(30));
                first_order.lock().unwrap().push("first:end");
            });
        });
        thread::sleep(Duration::from_millis(5));
        let second_order = order.clone();
        let second = thread::spawn(move || {
            with_file_mutation_queue(&path, || {
                second_order.lock().unwrap().push("second:start");
                second_order.lock().unwrap().push("second:end");
            });
        });
        first.join().unwrap();
        second.join().unwrap();
        assert_eq!(
            *order.lock().unwrap(),
            ["first:start", "first:end", "second:start", "second:end"]
        );
    }

    #[test]
    fn allows_different_files_to_proceed_in_parallel() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let dir = tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        let a_order = order.clone();
        let b_order = order.clone();
        let left = thread::spawn(move || {
            with_file_mutation_queue(&a, || {
                a_order.lock().unwrap().push("a:start");
                thread::sleep(Duration::from_millis(30));
                a_order.lock().unwrap().push("a:end");
            });
        });
        let right = thread::spawn(move || {
            with_file_mutation_queue(&b, || {
                b_order.lock().unwrap().push("b:start");
                thread::sleep(Duration::from_millis(30));
                b_order.lock().unwrap().push("b:end");
            });
        });
        left.join().unwrap();
        right.join().unwrap();
        let seen = order.lock().unwrap().clone();
        assert!(
            seen.iter().position(|s| *s == "a:start") < seen.iter().position(|s| *s == "a:end")
        );
        assert!(
            seen.iter().position(|s| *s == "b:start") < seen.iter().position(|s| *s == "b:end")
        );
        assert!(
            seen.iter().position(|s| *s == "b:start") < seen.iter().position(|s| *s == "a:end")
        );
    }

    #[test]
    fn uses_the_same_queue_for_symlink_aliases() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("target.txt");
        let alias = dir.path().join("alias.txt");
        std::fs::write(&target, "hello\n").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &alias).unwrap();
        #[cfg(windows)]
        if std::os::windows::fs::symlink_file(&target, &alias).is_err() {
            // Creating symlinks requires a privilege that is commonly unavailable
            // in Windows CI and developer environments; skip this capability test.
            return;
        }
        assert_eq!(mutation_queue_key(&target), mutation_queue_key(&alias));
        let order = Arc::new(Mutex::new(Vec::new()));
        let first_order = order.clone();
        let first = thread::spawn(move || {
            with_file_mutation_queue(&target, || {
                first_order.lock().unwrap().push("target:start");
                thread::sleep(Duration::from_millis(30));
                first_order.lock().unwrap().push("target:end");
            });
        });
        thread::sleep(Duration::from_millis(5));
        let second_order = order.clone();
        let second = thread::spawn(move || {
            with_file_mutation_queue(&alias, || {
                second_order.lock().unwrap().push("alias:start");
                second_order.lock().unwrap().push("alias:end");
            });
        });
        first.join().unwrap();
        second.join().unwrap();
        assert_eq!(
            *order.lock().unwrap(),
            ["target:start", "target:end", "alias:start", "alias:end"]
        );
    }
}
