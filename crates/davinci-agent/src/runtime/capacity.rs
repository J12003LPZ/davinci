//! Native completion admission; no upstream TypeScript counterpart.
//! In-process permits cover transport callbacks. Security-scan slots also take
//! exclusive files under a bound directory so two `davinci` processes share the
//! same two-scan cap.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::Duration;

pub use super::root_budget::{BudgetLimits, BudgetSnapshot, RootBudget};

const MAX_TOTAL: usize = 4;
const MAX_SCANS: usize = 2;

#[derive(Clone, Copy)]
pub enum RequestClass {
    Foreground,
    SecurityScan,
}

#[derive(Default)]
struct Counts {
    total: usize,
    scans: usize,
    waiters: VecDeque<(Weak<()>, bool)>,
}

pub struct RequestCapacity {
    counts: Mutex<Counts>,
    changed: Condvar,
}

pub static REQUEST_CAPACITY: RequestCapacity = RequestCapacity::new();

static SHARED_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Bind the cross-process scan-slot directory. Called from the host before a
/// security review; isolated `RequestCapacity::new()` pools ignore it.
pub fn bind_shared_directory(path: PathBuf) {
    *SHARED_DIR.lock().unwrap_or_else(|e| e.into_inner()) = Some(path);
}

fn shared_directory() -> Option<PathBuf> {
    SHARED_DIR.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

impl Default for RequestCapacity {
    fn default() -> Self {
        Self::new()
    }
}

impl RequestCapacity {
    pub const fn new() -> Self {
        Self {
            counts: Mutex::new(Counts {
                total: 0,
                scans: 0,
                waiters: VecDeque::new(),
            }),
            changed: Condvar::new(),
        }
    }

    /// Cancellation is checked even when a slot is immediately available.
    pub fn acquire(
        &self,
        class: RequestClass,
        cancelled: impl Fn() -> bool,
    ) -> Option<RequestPermit<'_>> {
        let dir = if std::ptr::eq(self, &REQUEST_CAPACITY) {
            shared_directory()
        } else {
            None
        };
        self.acquire_with_shared(class, cancelled, dir.as_deref())
    }

    pub fn acquire_with_shared(
        &self,
        class: RequestClass,
        cancelled: impl Fn() -> bool,
        shared: Option<&Path>,
    ) -> Option<RequestPermit<'_>> {
        let scan = matches!(class, RequestClass::SecurityScan);
        // A weak ticket disappears even when a cancellation callback panics.
        let ticket = Arc::new(());
        let weak = Arc::downgrade(&ticket);
        let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        counts.waiters.push_back((weak.clone(), scan));
        loop {
            counts
                .waiters
                .retain(|(ticket, _)| ticket.strong_count() > 0);
            if cancelled() {
                counts.waiters.retain(|(ticket, _)| !ticket.ptr_eq(&weak));
                self.changed.notify_all();
                return None;
            }
            // FIFO among eligible requests. A scan waiting for its class cap
            // cannot obstruct the foreground slots reserved by MAX_SCANS.
            let first = counts
                .waiters
                .iter()
                .position(|(_, scan)| !scan || counts.scans < MAX_SCANS);
            if counts.total < MAX_TOTAL && first.is_some_and(|i| counts.waiters[i].0.ptr_eq(&weak))
            {
                counts.waiters.remove(first.expect("eligible ticket"));
                counts.total += 1;
                counts.scans += usize::from(scan);
                self.changed.notify_all();
                drop(counts);
                let mut permit = RequestPermit {
                    pool: self,
                    class,
                    shared: None,
                };
                if scan {
                    if let Some(dir) = shared {
                        permit.shared = Some(acquire_scan_file(dir, &cancelled)?);
                    }
                }
                return Some(permit);
            }
            counts = self
                .changed
                .wait_timeout(counts, Duration::from_millis(20))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}

fn acquire_scan_file(dir: &Path, cancelled: &dyn Fn() -> bool) -> Option<File> {
    let _ = std::fs::create_dir_all(dir);
    loop {
        if cancelled() {
            return None;
        }
        for slot in 0..MAX_SCANS {
            if let Some(file) = try_lock_scan_slot(dir, slot) {
                return Some(file);
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn try_lock_scan_slot(dir: &Path, slot: usize) -> Option<File> {
    let path = dir.join(format!("scan-{slot}.lock"));
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    let file = options.open(&path).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        // LOCK_EX | LOCK_NB. Linked against the system C library; no crate dep.
        extern "C" {
            fn flock(fd: i32, operation: i32) -> i32;
        }
        const LOCK_EX: i32 = 2;
        const LOCK_NB: i32 = 4;
        if unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) } != 0 {
            return None;
        }
    }
    Some(file)
}

pub struct RequestPermit<'a> {
    pool: &'a RequestCapacity,
    class: RequestClass,
    shared: Option<File>,
}

impl Drop for RequestPermit<'_> {
    fn drop(&mut self) {
        self.shared.take();
        let mut counts = self.pool.counts.lock().unwrap_or_else(|e| e.into_inner());
        counts.total -= 1;
        counts.scans -= usize::from(matches!(self.class, RequestClass::SecurityScan));
        self.pool.changed.notify_all();
    }
}

/// Dedicated worker slot capacity allocator supporting dynamic concurrency limits and atomic acquisition.
pub struct WorkerSlotCapacity {
    active: Mutex<WorkerCounts>,
    changed: Condvar,
}

struct WorkerCounts {
    active: usize,
    /// The limit each running permit was admitted under, as limit -> count.
    /// A caller asking for limit 1 (a serial barrier) must also keep later
    /// callers with a larger limit out while it runs, so admission respects
    /// the smallest limit held, not only the caller's own.
    held_limits: std::collections::BTreeMap<usize, usize>,
    waiters: VecDeque<Weak<()>>,
}

impl Default for WorkerSlotCapacity {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkerSlotCapacity {
    pub const fn new() -> Self {
        Self {
            active: Mutex::new(WorkerCounts {
                active: 0,
                held_limits: std::collections::BTreeMap::new(),
                waiters: VecDeque::new(),
            }),
            changed: Condvar::new(),
        }
    }

    pub fn acquire(
        &self,
        max_concurrency: usize,
        cancelled: impl Fn() -> bool,
    ) -> Option<WorkerSlotPermit<'_>> {
        if max_concurrency == 0 {
            return None;
        }
        let ticket = Arc::new(());
        let weak = Arc::downgrade(&ticket);
        let mut active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        active.waiters.push_back(weak.clone());
        loop {
            active.waiters.retain(|ticket| ticket.strong_count() > 0);
            if cancelled() {
                active.waiters.retain(|ticket| !ticket.ptr_eq(&weak));
                self.changed.notify_all();
                return None;
            }
            let ceiling = active
                .held_limits
                .keys()
                .next()
                .map_or(max_concurrency, |held| max_concurrency.min(*held));
            if active.active < ceiling
                && active
                    .waiters
                    .front()
                    .is_some_and(|ticket| ticket.ptr_eq(&weak))
            {
                active.waiters.pop_front();
                active.active += 1;
                *active.held_limits.entry(max_concurrency).or_default() += 1;
                self.changed.notify_all();
                return Some(WorkerSlotPermit {
                    pool: self,
                    limit: max_concurrency,
                });
            }
            let (next_active, _timeout) = self
                .changed
                .wait_timeout(active, Duration::from_millis(20))
                .unwrap_or_else(|e| e.into_inner());
            active = next_active;
        }
    }

    pub fn active_count(&self) -> usize {
        self.active.lock().unwrap_or_else(|e| e.into_inner()).active
    }
}

pub struct WorkerSlotPermit<'a> {
    pool: &'a WorkerSlotCapacity,
    limit: usize,
}

impl Drop for WorkerSlotPermit<'_> {
    fn drop(&mut self) {
        let mut active = self.pool.active.lock().unwrap_or_else(|e| e.into_inner());
        active.active = active.active.saturating_sub(1);
        if let Some(count) = active.held_limits.get_mut(&self.limit) {
            *count -= 1;
            if *count == 0 {
                active.held_limits.remove(&self.limit);
            }
        }
        self.pool.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Try to acquire with `limit` for `wait`, reporting whether it got in.
    fn admitted_within(pool: &WorkerSlotCapacity, limit: usize, wait: Duration) -> bool {
        let deadline = std::time::Instant::now() + wait;
        pool.acquire(limit, || std::time::Instant::now() >= deadline)
            .is_some()
    }

    #[test]
    fn a_serial_holder_keeps_parallel_callers_out_and_vice_versa() {
        let pool = WorkerSlotCapacity::new();
        let short = Duration::from_millis(100);

        // A serial (limit 1) child is running: a parallel-safe caller with a
        // larger limit must wait for it, not join it.
        let serial = pool.acquire(1, || false).unwrap();
        assert!(!admitted_within(&pool, 4, short));
        assert_eq!(pool.active_count(), 1);
        drop(serial);
        assert!(admitted_within(&pool, 4, short));

        // Parallel-safe children share; a serial caller waits for all of them.
        let first = pool.acquire(4, || false).unwrap();
        let second = pool.acquire(4, || false).unwrap();
        assert_eq!(pool.active_count(), 2);
        assert!(!admitted_within(&pool, 1, short));
        drop(first);
        assert!(!admitted_within(&pool, 1, short));
        drop(second);
        assert!(admitted_within(&pool, 1, short));
        assert_eq!(pool.active_count(), 0);
    }

    #[test]
    fn harness_worker_waiters_are_fifo_and_zero_capacity_refuses() {
        let pool = WorkerSlotCapacity::new();
        assert!(pool.acquire(0, || false).is_none());
        let occupied = pool.acquire(1, || false).unwrap();
        std::thread::scope(|scope| {
            let (ready_tx, ready_rx) = std::sync::mpsc::channel();
            let (order_tx, order_rx) = std::sync::mpsc::channel();
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            for index in 0..3 {
                let ready = ready_tx.clone();
                let order = order_tx.clone();
                let pool = &pool;
                scope.spawn(move || {
                    let announced = std::cell::Cell::new(false);
                    let permit = pool
                        .acquire(1, || {
                            if !announced.replace(true) {
                                ready.send(index).unwrap();
                            }
                            std::time::Instant::now() >= deadline
                        })
                        .expect("queued worker admitted before test deadline");
                    order.send(index).unwrap();
                    drop(permit);
                });
                assert_eq!(
                    ready_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
                    index
                );
            }
            drop(occupied);
            for index in 0..3 {
                assert_eq!(
                    order_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
                    index
                );
            }
        });
        assert_eq!(pool.active_count(), 0);
    }

    #[test]
    fn harness_class_blocked_scan_does_not_block_foreground() {
        let pool = RequestCapacity::new();
        let a = pool.acquire(RequestClass::SecurityScan, || false).unwrap();
        let b = pool.acquire(RequestClass::SecurityScan, || false).unwrap();
        let cancelled = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|scope| {
            let (tx, rx) = std::sync::mpsc::channel();
            let pool = &pool;
            let cancelled = &cancelled;
            scope.spawn(move || {
                assert!(pool
                    .acquire(RequestClass::SecurityScan, || {
                        let _ = tx.send(());
                        cancelled.load(std::sync::atomic::Ordering::SeqCst)
                    })
                    .is_none());
            });
            rx.recv_timeout(Duration::from_secs(2)).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            let foreground = pool.acquire(RequestClass::Foreground, || {
                std::time::Instant::now() >= deadline
            });
            cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
            assert!(foreground.is_some());
        });
        drop((a, b));
        assert_eq!(pool.counts.lock().unwrap().total, 0);
    }

    #[test]
    fn cancelled_request_never_receives_capacity() {
        assert!(RequestCapacity::new()
            .acquire(RequestClass::Foreground, || true)
            .is_none());
    }

    #[test]
    fn security_scan_shares_runtime_capacity() {
        let pool = RequestCapacity::new();
        let a = pool.acquire(RequestClass::SecurityScan, || false).unwrap();
        let b = pool.acquire(RequestClass::SecurityScan, || false).unwrap();
        assert!(pool.acquire(RequestClass::SecurityScan, || true).is_none());
        let foreground = pool.acquire(RequestClass::Foreground, || false).unwrap();
        let counts = pool.counts.lock().unwrap();
        assert_eq!((counts.total, counts.scans), (3, 2));
        drop(counts);
        drop((a, b, foreground));
    }

    #[test]
    fn scans_share_total_capacity_and_leave_foreground_room() {
        let pool = RequestCapacity::new();
        let a = pool.acquire(RequestClass::SecurityScan, || false).unwrap();
        let b = pool.acquire(RequestClass::SecurityScan, || false).unwrap();
        let c = pool.acquire(RequestClass::Foreground, || false).unwrap();
        let d = pool.acquire(RequestClass::Foreground, || false).unwrap();
        let counts = pool.counts.lock().unwrap();
        assert_eq!((counts.total, counts.scans), (4, 2));
        drop(counts);
        drop((a, b, c, d));
        assert_eq!(pool.counts.lock().unwrap().total, 0);
    }

    #[test]
    fn queued_scan_can_cancel_without_waiting_for_a_transport() {
        let pool = RequestCapacity::new();
        let _a = pool.acquire(RequestClass::SecurityScan, || false).unwrap();
        let _b = pool.acquire(RequestClass::SecurityScan, || false).unwrap();
        let polls = std::sync::atomic::AtomicUsize::new(0);
        assert!(pool
            .acquire(RequestClass::SecurityScan, || {
                polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 0
            })
            .is_none());
        assert_eq!(pool.counts.lock().unwrap().total, 2);
    }

    #[test]
    fn foreground_obeys_shared_limit_and_release_wakes_waiter() {
        let pool = RequestCapacity::new();
        let permits: Vec<_> = (0..4)
            .map(|_| pool.acquire(RequestClass::Foreground, || false).unwrap())
            .collect();
        let polls = std::sync::atomic::AtomicUsize::new(0);
        assert!(pool
            .acquire(RequestClass::Foreground, || polls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                > 0)
            .is_none());
        std::thread::scope(|scope| {
            let (tx, rx) = std::sync::mpsc::channel();
            let shared = &pool;
            let waiter = scope.spawn(move || {
                let _permit = shared
                    .acquire(RequestClass::SecurityScan, || {
                        let _ = tx.send(());
                        false
                    })
                    .unwrap();
            });
            rx.recv_timeout(Duration::from_secs(2)).unwrap();
            drop(permits);
            waiter.join().unwrap();
        });
        assert_eq!(pool.counts.lock().unwrap().total, 0);
    }

    #[test]
    fn callback_panic_releases_permit() {
        let pool = RequestCapacity::new();
        let _ = std::panic::catch_unwind(|| {
            let _permit = pool.acquire(RequestClass::SecurityScan, || false).unwrap();
            panic!("fixture transport panic");
        });
        let counts = pool.counts.lock().unwrap();
        assert_eq!((counts.total, counts.scans), (0, 0));
    }

    #[test]
    fn security_scan_shared_lock_rejects_third_slot() {
        let dir = tempfile::tempdir().unwrap();
        let pool = RequestCapacity::new();
        let a = pool
            .acquire_with_shared(RequestClass::SecurityScan, || false, Some(dir.path()))
            .unwrap();
        let b = pool
            .acquire_with_shared(RequestClass::SecurityScan, || false, Some(dir.path()))
            .unwrap();
        let polls = std::sync::atomic::AtomicUsize::new(0);
        assert!(pool
            .acquire_with_shared(
                RequestClass::SecurityScan,
                || polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 2,
                Some(dir.path())
            )
            .is_none());
        drop((a, b));
        assert!(pool
            .acquire_with_shared(RequestClass::SecurityScan, || false, Some(dir.path()))
            .is_some());
    }

    #[test]
    fn security_scan_capacity_is_shared_across_processes() {
        if let Ok(dir) = std::env::var("DAVINCI_CAPACITY_CHILD") {
            let pool = RequestCapacity::new();
            let path = PathBuf::from(dir);
            let _a = pool
                .acquire_with_shared(RequestClass::SecurityScan, || false, Some(&path))
                .expect("child first scan slot");
            let _b = pool
                .acquire_with_shared(RequestClass::SecurityScan, || false, Some(&path))
                .expect("child second scan slot");
            std::fs::write(path.join("ready"), b"1").unwrap();
            let release = path.join("release");
            while !release.exists() {
                std::thread::sleep(Duration::from_millis(20));
            }
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let child_stdout = std::fs::File::create(dir.path().join("child-stdout.log")).unwrap();
        let child_stderr = std::fs::File::create(dir.path().join("child-stderr.log")).unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .env("DAVINCI_CAPACITY_CHILD", dir.path())
            .env("RUST_TEST_THREADS", "1")
            .args([
                "runtime::capacity::tests::security_scan_capacity_is_shared_across_processes",
                "--exact",
                "--nocapture",
            ])
            .stdout(child_stdout)
            .stderr(child_stderr)
            .spawn()
            .unwrap();
        let ready = dir.path().join("ready");
        let started = std::time::Instant::now();
        while !ready.exists() {
            if let Ok(Some(status)) = child.try_wait() {
                panic!(
                    "capacity child exited early {status:?} stdout={} stderr={}",
                    std::fs::read_to_string(dir.path().join("child-stdout.log"))
                        .unwrap_or_default(),
                    std::fs::read_to_string(dir.path().join("child-stderr.log"))
                        .unwrap_or_default()
                );
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "child did not take scan slots stdout={} stderr={}",
                std::fs::read_to_string(dir.path().join("child-stdout.log")).unwrap_or_default(),
                std::fs::read_to_string(dir.path().join("child-stderr.log")).unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let pool = RequestCapacity::new();
        let polls = std::sync::atomic::AtomicUsize::new(0);
        assert!(pool
            .acquire_with_shared(
                RequestClass::SecurityScan,
                || polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 2,
                Some(dir.path())
            )
            .is_none());
        std::fs::write(dir.path().join("release"), b"1").unwrap();
        let status = child.wait().unwrap();
        assert!(
            status.success(),
            "child={status:?} stdout={} stderr={}",
            std::fs::read_to_string(dir.path().join("child-stdout.log")).unwrap_or_default(),
            std::fs::read_to_string(dir.path().join("child-stderr.log")).unwrap_or_default()
        );
    }

    #[test]
    fn worker_slot_capacity_enforces_limit_and_releases() {
        let pool = WorkerSlotCapacity::new();
        let slot1 = pool.acquire(2, || false).unwrap();
        let slot2 = pool.acquire(2, || false).unwrap();
        assert_eq!(pool.active_count(), 2);

        // Third slot cancelled immediately
        assert!(pool.acquire(2, || true).is_none());

        // Drop one slot, acquire succeeds
        drop(slot1);
        assert_eq!(pool.active_count(), 1);
        let slot3 = pool.acquire(2, || false).unwrap();
        assert_eq!(pool.active_count(), 2);

        drop(slot2);
        drop(slot3);
        assert_eq!(pool.active_count(), 0);
    }
}
