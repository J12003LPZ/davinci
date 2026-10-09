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

/// Removes a path's queue once its last user is done, as the TS queue
/// deletes its entry in `finally`. Every user clones the `Arc` while holding
/// the map lock, so a count of two (map + this release) under that lock
/// means nobody else holds or waits for it.
struct QueueRelease {
    key: String,
    lock: Arc<Mutex<()>>,
}

impl Drop for QueueRelease {
    fn drop(&mut self) {
        let mut map = queues().lock().unwrap_or_else(|err| err.into_inner());
        if Arc::strong_count(&self.lock) == 2
            && map
                .get(&self.key)
                .is_some_and(|current| Arc::ptr_eq(current, &self.lock))
        {
            map.remove(&self.key);
        }
    }
}

/// Hold the per-file lock for the duration of `fn`.
pub fn with_file_mutation_queue<T>(file_path: &Path, func: impl FnOnce() -> T) -> T {
    let key = mutation_queue_key(file_path);
    let lock = {
        let mut map = queues().lock().unwrap_or_else(|err| err.into_inner());
        map.entry(key.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let release = QueueRelease { key, lock };
    let guard = release.lock.lock().unwrap_or_else(|err| err.into_inner());
    let result = func();
    drop(guard);
    drop(release);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn completed_file_queues_are_released() {
        let dir = tempdir().unwrap();
        let paths: Vec<_> = (0..32)
            .map(|n| dir.path().join(format!("released-{n}.txt")))
            .collect();
        for path in &paths {
            with_file_mutation_queue(path, || ());
        }
        let map = queues().lock().unwrap();
        let retained = paths
            .iter()
            .filter(|path| map.contains_key(&mutation_queue_key(path)))
            .count();
        assert_eq!(retained, 0);
    }

    #[test]
    fn serializes_operations_for_the_same_file() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let dir = tempdir().unwrap();
        let path = dir.path().join("same.txt");
        let first_order = order.clone();
        let first_path = path.clone();
        // The second operation starts only once the first holds the queue,
        // so the order is fixed by the queue, not by scheduler timing.
        let (entered, holding) = std::sync::mpsc::channel();
        let first = thread::spawn(move || {
            with_file_mutation_queue(&first_path, || {
                first_order.lock().unwrap().push("first:start");
                entered.send(()).unwrap();
                thread::sleep(Duration::from_millis(30));
                first_order.lock().unwrap().push("first:end");
            });
        });
        holding.recv().unwrap();
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
