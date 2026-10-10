//! Hierarchical cancellation tokens for agents, workers, and background jobs.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

type CancellationCallback = Box<dyn Fn() + Send + Sync + 'static>;
type ChildList = Arc<Mutex<Vec<Weak<CancellationInner>>>>;
type CallbackList = Arc<Mutex<Vec<CancellationCallback>>>;

/// Child-list length at which `child_token` starts pruning expired registrations.
const PRUNE_FLOOR: usize = 32;

/// Inner state tracked for a cancellation token node.
pub struct CancellationInner {
    pub inner: Arc<AtomicBool>,
    /// Once-only guard for callbacks and child propagation. Kept apart from
    /// `inner` because `as_atomic_bool` hands that flag out writable: a raw
    /// store must not consume the cleanup that a later `cancel()` owes.
    propagated: Arc<AtomicBool>,
    pub children: ChildList,
    pub callbacks: CallbackList,
}

/// Hierarchical cancellation token.
/// Cancelling a parent propagates downward to all active children and attached callbacks.
/// Cancelling a child does not cancel the parent.
#[derive(Clone)]
pub struct CancellationToken {
    inner: Arc<AtomicBool>,
    propagated: Arc<AtomicBool>,
    children: ChildList,
    callbacks: CallbackList,
    node: Arc<CancellationInner>,
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CancellationToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

impl CancellationToken {
    /// Create a new, independent root cancellation token.
    pub fn new() -> Self {
        let inner = Arc::new(AtomicBool::new(false));
        let propagated = Arc::new(AtomicBool::new(false));
        let children = Arc::new(Mutex::new(Vec::new()));
        let callbacks = Arc::new(Mutex::new(Vec::new()));
        let node = Arc::new(CancellationInner {
            inner: inner.clone(),
            propagated: propagated.clone(),
            children: children.clone(),
            callbacks: callbacks.clone(),
        });
        Self {
            inner,
            propagated,
            children,
            callbacks,
            node,
        }
    }

    /// Check if this token has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.inner.load(Ordering::SeqCst)
    }

    /// Exposes an `Arc<AtomicBool>` view suitable for APIs expecting a raw flag (e.g. `davinci-ai`).
    ///
    /// Storing `true` here makes the token read as cancelled but runs no
    /// cleanup; a later `cancel()` still fires callbacks and reaches children.
    pub fn as_atomic_bool(&self) -> Arc<AtomicBool> {
        self.inner.clone()
    }

    /// Cancel this token and propagate downward to all attached child tokens and callbacks.
    pub fn cancel(&self) {
        self.inner.store(true, Ordering::SeqCst);
        if self.propagated.swap(true, Ordering::SeqCst) {
            // Already propagated
            return;
        }

        // Fire callbacks
        let callbacks = {
            let mut guard = self.callbacks.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *guard)
        };
        // Cleanup must reach every callback and child even when one panics.
        // Re-raise the first panic after propagation to preserve observability.
        let mut panic = None;
        for cb in callbacks {
            if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(cb)) {
                if panic.is_none() {
                    panic = Some(payload);
                }
            }
        }

        // Propagate to child tokens
        let children: Vec<Arc<CancellationInner>> = {
            let mut guard = self.children.lock().unwrap_or_else(|e| e.into_inner());
            let list = guard.iter().filter_map(|w| w.upgrade()).collect();
            guard.retain(|w| w.strong_count() > 0);
            list
        };
        for child_node in children {
            let child_token = CancellationToken {
                inner: child_node.inner.clone(),
                propagated: child_node.propagated.clone(),
                children: child_node.children.clone(),
                callbacks: child_node.callbacks.clone(),
                node: child_node,
            };
            if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                child_token.cancel();
            })) {
                if panic.is_none() {
                    panic = Some(payload);
                }
            }
        }
        if let Some(payload) = panic {
            std::panic::resume_unwind(payload);
        }
    }

    /// Creates a child token attached to this parent.
    /// When this parent is cancelled, the child will also be cancelled.
    /// Cancelling the child does not cancel this parent.
    pub fn child_token(&self) -> Self {
        let child = Self::new();
        // Read the flag under the children lock. `cancel` sets the flag before
        // it takes this lock to drain the list, so either we see the flag here
        // or `cancel` sees our registration. Checking outside the lock let a
        // cancel run entirely between the check and the push (WOR-52).
        let mut guard = self.children.lock().unwrap_or_else(|e| e.into_inner());
        if self.is_cancelled() {
            drop(guard);
            child.cancel();
        } else {
            after_flag_check_hook();
            // Prune expired registrations whenever the list length hits a power of two
            // (>= PRUNE_FLOOR). Amortized O(1) per child, and the list stays within
            // 2x the live children plus PRUNE_FLOOR without waiting for root cancel.
            let len = guard.len();
            if len >= PRUNE_FLOOR && len.is_power_of_two() {
                guard.retain(|w| w.strong_count() > 0);
            }
            guard.push(Arc::downgrade(&child.node));
        }
        child
    }

    /// Creates a detached child token.
    /// Detached means it does NOT inherit cancellation from `self` (e.g. a parent turn),
    /// but if `ancestor` (e.g. a session or run root) is supplied, it inherits from `ancestor`.
    pub fn detached_child(&self, ancestor: Option<&CancellationToken>) -> Self {
        if let Some(anc) = ancestor {
            anc.child_token()
        } else {
            Self::new()
        }
    }

    /// Register a callback to execute immediately when cancelled, or synchronously now if already cancelled.
    pub fn on_cancel<F>(&self, f: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        let mut guard = self.callbacks.lock().unwrap_or_else(|e| e.into_inner());
        if self.is_cancelled() {
            drop(guard);
            f();
        } else {
            guard.push(Box::new(f));
        }
    }

    /// Attach a `JobBook` so that cancelling this token immediately executes `JobBook::kill_all()`.
    pub fn attach_job_book(&self, jobs: Arc<Mutex<crate::jobs::JobBook>>) {
        self.on_cancel(move || {
            if let Ok(mut book) = jobs.lock() {
                book.kill_all();
            }
        });
    }

    /// Bind the host's shared job book to its current runtime only.
    pub(crate) fn bind_job_book(&self, jobs: &Arc<Mutex<crate::jobs::JobBook>>) {
        let owner = Arc::downgrade(&self.inner);
        {
            let mut book = jobs.lock().unwrap_or_else(|e| e.into_inner());
            if book
                .cancellation_owner
                .as_ref()
                .is_some_and(|current| current.ptr_eq(&owner))
            {
                return;
            }
            book.cancellation_owner = Some(owner.clone());
        }
        let jobs = Arc::downgrade(jobs);
        self.on_cancel(move || {
            if let Some(jobs) = jobs.upgrade() {
                let mut book = jobs.lock().unwrap_or_else(|e| e.into_inner());
                if book
                    .cancellation_owner
                    .as_ref()
                    .is_some_and(|current| current.ptr_eq(&owner))
                {
                    book.kill_all();
                }
            }
        });
    }
}

#[cfg(test)]
thread_local! {
    static AFTER_FLAG_CHECK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

/// Test seam: runs once, on the calling thread, after `child_token` has seen
/// the parent un-cancelled and before the child is registered.
fn after_flag_check_hook() {
    #[cfg(test)]
    if let Some(hook) = AFTER_FLAG_CHECK.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panicking_callbacks_do_not_skip_cleanup_or_child_propagation() {
        let parent = CancellationToken::new();
        let first = parent.child_token();
        let second = parent.child_token();
        let grandchild = first.child_token();
        let cleaned = Arc::new(AtomicBool::new(false));
        parent.on_cancel(|| panic!("synthetic parent callback failure"));
        first.on_cancel(|| panic!("synthetic child callback failure"));
        let flag = cleaned.clone();
        parent.on_cancel(move || {
            flag.store(true, Ordering::SeqCst);
        });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| parent.cancel()));
        assert!(result.is_err(), "callback panic must remain observable");
        assert!(cleaned.load(Ordering::SeqCst));
        assert!(first.is_cancelled());
        assert!(second.is_cancelled());
        assert!(grandchild.is_cancelled());
        parent.cancel();
    }

    /// WOR-52: a child registered while the parent is mid-cancel must still
    /// end up cancelled. Before the fix `child_token` read the flag outside
    /// the children lock, so a cancel that swapped the flag and drained the
    /// list in between left the late child live forever.
    #[test]
    fn wor52_child_created_during_parent_cancel_is_never_missed() {
        let parent = CancellationToken::new();
        std::thread::scope(|scope| {
            let (go, wait) = std::sync::mpsc::channel::<()>();
            let target = &parent;
            let canceller = scope.spawn(move || {
                wait.recv().unwrap();
                target.cancel();
            });
            // Force the interleaving: parent.cancel() starts exactly between
            // child_token's flag check and its registration.
            let probe = parent.clone();
            AFTER_FLAG_CHECK.with(|slot| {
                *slot.borrow_mut() = Some(Box::new(move || {
                    go.send(()).unwrap();
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                    while !probe.is_cancelled() && std::time::Instant::now() < deadline {
                        std::thread::yield_now();
                    }
                    // Give a lock-free cancel() time to finish draining.
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }));
            });
            let child = parent.child_token();
            canceller.join().unwrap();
            assert!(parent.is_cancelled());
            assert!(
                child.is_cancelled(),
                "child created concurrently with parent.cancel() missed propagation"
            );
        });
    }

    #[test]
    fn wor52_child_token_race_stress() {
        for _ in 0..2_000 {
            let parent = CancellationToken::new();
            let start = std::sync::Barrier::new(2);
            let child = std::thread::scope(|scope| {
                let creator = scope.spawn(|| {
                    start.wait();
                    parent.child_token()
                });
                start.wait();
                parent.cancel();
                creator.join().unwrap()
            });
            assert!(child.is_cancelled());
        }
    }

    #[test]
    fn audit_finished_child_registrations_remain_bounded() {
        let parent = CancellationToken::new();
        for _ in 0..10_000 {
            let child = parent.child_token();
            child.cancel();
            drop(child);
        }
        let registered = parent.children.lock().unwrap();
        let live = registered
            .iter()
            .filter(|child| child.strong_count() > 0)
            .count();
        assert_eq!(live, 0);
        assert!(
            registered.len() <= 128,
            "{} expired child registrations remain with no live children",
            registered.len()
        );
    }

    #[test]
    fn pruning_keeps_live_children_cancellable() {
        let parent = CancellationToken::new();
        let mut live = Vec::new();
        for i in 0..1_000 {
            let child = parent.child_token();
            if i % 10 == 0 {
                live.push(child);
            }
        }
        assert_eq!(live.len(), 100);
        assert!(
            parent.children.lock().unwrap().len() <= 2 * live.len() + 2 * PRUNE_FLOOR,
            "registrations must stay within a small multiple of live children"
        );
        parent.cancel();
        assert!(live.iter().all(CancellationToken::is_cancelled));
    }

    #[test]
    fn raw_abort_flag_still_lets_cancel_reach_children_and_callbacks() {
        let parent = CancellationToken::new();
        let child = parent.child_token();
        let grandchild = child.child_token();
        let callbacks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = callbacks.clone();
        parent.on_cancel(move || {
            observed.fetch_add(1, Ordering::SeqCst);
        });

        // tool_durability_failure and every `abort_signal` holder can store
        // into this flag before the runtime's own cleanup runs.
        parent.as_atomic_bool().store(true, Ordering::SeqCst);
        assert!(parent.is_cancelled());
        assert!(!child.is_cancelled(), "a raw store is not propagation");

        parent.cancel();
        parent.cancel();

        assert!(child.is_cancelled() && grandchild.is_cancelled());
        assert_eq!(callbacks.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn raw_abort_flag_runs_late_callbacks_once() {
        let token = CancellationToken::new();
        token.as_atomic_bool().store(true, Ordering::SeqCst);
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = count.clone();
        token.on_cancel(move || {
            observed.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(count.load(Ordering::SeqCst), 1, "runs at registration");
        token.cancel();
        assert_eq!(count.load(Ordering::SeqCst), 1, "not run again by cancel");
        assert!(token.child_token().is_cancelled());
    }

    #[test]
    fn f03_job_binding_is_bounded_and_does_not_retain_book() {
        let token = CancellationToken::new();
        let jobs = Arc::new(Mutex::new(crate::jobs::JobBook::default()));
        let weak = Arc::downgrade(&jobs);
        token.bind_job_book(&jobs);
        token.bind_job_book(&jobs);
        assert_eq!(token.callbacks.lock().unwrap().len(), 1);
        drop(jobs);
        assert!(weak.upgrade().is_none());
        token.cancel();
    }

    #[test]
    fn cancellation_registration_runs_once_across_cancel_race() {
        for _ in 0..64 {
            let token = CancellationToken::new();
            let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let start = Arc::new(std::sync::Barrier::new(2));
            std::thread::scope(|scope| {
                let token = &token;
                let start = &start;
                let count = count.clone();
                scope.spawn(move || {
                    start.wait();
                    token.on_cancel(move || {
                        count.fetch_add(1, Ordering::SeqCst);
                    });
                });
                start.wait();
                token.cancel();
            });
            assert_eq!(count.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn test_parent_cancellation_reaches_child() {
        let parent = CancellationToken::new();
        let child1 = parent.child_token();
        let child2 = child1.child_token();

        assert!(!parent.is_cancelled());
        assert!(!child1.is_cancelled());
        assert!(!child2.is_cancelled());

        parent.cancel();

        assert!(parent.is_cancelled());
        assert!(child1.is_cancelled());
        assert!(child2.is_cancelled());
        assert!(child2.as_atomic_bool().load(Ordering::SeqCst));
    }

    #[test]
    fn test_child_cancel_does_not_cancel_parent() {
        let parent = CancellationToken::new();
        let child = parent.child_token();

        child.cancel();

        assert!(child.is_cancelled());
        assert!(!parent.is_cancelled());
    }

    #[test]
    fn test_detached_child_cancellation() {
        let root = CancellationToken::new();
        let turn = root.child_token();
        let detached = turn.detached_child(Some(&root));

        assert!(!root.is_cancelled());
        assert!(!turn.is_cancelled());
        assert!(!detached.is_cancelled());

        // Cancelling the turn does NOT cancel the detached child
        turn.cancel();
        assert!(turn.is_cancelled());
        assert!(!detached.is_cancelled());
        assert!(!root.is_cancelled());

        // Cancelling the root DOES cancel the detached child
        root.cancel();
        assert!(root.is_cancelled());
        assert!(detached.is_cancelled());
    }

    #[test]
    fn test_cancellation_reaches_jobs() {
        let token = CancellationToken::new();
        let jobs = Arc::new(Mutex::new(crate::jobs::JobBook::default()));
        token.attach_job_book(jobs.clone());

        assert_eq!(jobs.lock().unwrap().running(), 0);
        token.cancel();
        // jobs was notified and kill_all executed without panics
        assert!(token.is_cancelled());
    }
}
