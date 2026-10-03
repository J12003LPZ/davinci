use davinci_coding_agent::native_extensions::security_scan::controller::ScanCoordinator;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

struct CleanupFlag(Arc<AtomicBool>);

impl Drop for CleanupFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[test]
fn wait_does_not_return_before_worker_cleanup_finishes() {
    let coordinator = ScanCoordinator::default();
    let cleanup_done = Arc::new(AtomicBool::new(false));
    let cleanup_done_worker = cleanup_done.clone();
    let (finished_tx, finished_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();

    coordinator
        .start(move |run| {
            let _cleanup = CleanupFlag(cleanup_done_worker);
            run.finish(Ok(true));
            finished_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        })
        .unwrap();

    finished_rx.recv_timeout(Duration::from_secs(2)).unwrap();

    let waiter = coordinator.clone();
    let waiter_done = Arc::new(AtomicBool::new(false));
    let waiter_done_thread = waiter_done.clone();
    let waiter_thread = std::thread::spawn(move || {
        waiter.wait();
        waiter_done_thread.store(true, Ordering::Release);
    });

    std::thread::sleep(Duration::from_millis(50));
    let returned_early = waiter_done.load(Ordering::Acquire);

    release_tx.send(()).unwrap();
    waiter_thread.join().unwrap();

    assert!(cleanup_done.load(Ordering::Acquire));
    assert!(
        !returned_early,
        "wait returned after terminal status but before the worker released its resources"
    );
}
